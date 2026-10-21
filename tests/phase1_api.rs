//! 一期后端验收：注册、会话、页内确认、报餐覆盖与锁定、食堂范围、评分、强提醒、渠道顺序、推送预留。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
use duolinzao::config::{example_config, AppConfig};
use duolinzao::router;
use duolinzao::state::{AppState, CapturingMailer, ManualClock, ScriptedBackend};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static GATE: Mutex<()> = Mutex::const_new(());

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

const EDGE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0";

struct Resp {
    status: StatusCode,
    bytes: Vec<u8>,
    cookie: Option<String>,
}

impl Resp {
    fn json(&self) -> Value {
        if self.bytes.is_empty() {
            return Value::Null;
        }
        serde_json::from_slice(&self.bytes)
            .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&self.bytes).into_owned()}))
    }

    fn token(&self) -> String {
        let header = self.cookie.as_deref().expect("缺少会话 cookie");
        assert!(header.contains("HttpOnly"), "{header}");
        header
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_string()
    }
}

struct Api {
    app: axum::Router,
    pool: PgPool,
    mailer: Arc<CapturingMailer>,
    scripts: ScriptedBackend,
}

impl Api {
    async fn call(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
        user_agent: Option<&str>,
        ip: Option<SocketAddr>,
    ) -> Resp {
        let mut builder = Request::builder().method(method).uri(uri);
        // 与前端 api() 一致：所有请求都带 CSRF 标记头。
        builder = builder.header("x-dlz-requested", "1");
        if let Some(token) = token {
            builder = builder.header("cookie", format!("dlz_session={token}"));
        }
        builder = builder.header("user-agent", user_agent.unwrap_or("phase1-test"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let payload = body
            .map(|value| serde_json::to_vec(&value).unwrap())
            .unwrap_or_default();
        let mut request = builder.body(Body::from(payload)).unwrap();
        if let Some(ip) = ip {
            request.extensions_mut().insert(ConnectInfo(ip));
        }
        let response = self.app.clone().oneshot(request).await.expect("oneshot");
        let status = response.status();
        let cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Resp {
            status,
            bytes: bytes.to_vec(),
            cookie,
        }
    }

    async fn upload(&self, uri: &str, token: &str, bytes: &[u8]) -> Resp {
        let boundary = "phase1boundary";
        let mut payload = Vec::new();
        payload.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"dish.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        payload.extend_from_slice(bytes);
        payload.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header("x-dlz-requested", "1")
            .header("cookie", format!("dlz_session={token}"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(payload))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.expect("upload");
        Resp {
            status: response.status(),
            bytes: response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
            cookie: None,
        }
    }
}

fn must(res: Resp, status: StatusCode, label: &str) -> Resp {
    assert_eq!(res.status, status, "{label}: {}", res.json());
    res
}

/// 从邮箱正文里抠 6 位验证码（正文不含链接，只有「10 分钟」这类短数字）。
fn otp_code(body: &str) -> String {
    body.split(|item: char| !item.is_ascii_digit())
        .find(|part| part.len() == 6)
        .expect("邮件正文应含 6 位验证码")
        .to_string()
}

fn uid(value: &Value, key: &str) -> Uuid {
    Uuid::parse_str(
        value[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} in {value}")),
    )
    .unwrap()
}

fn assert_groups(groups: &Value, label: &str) {
    let object = groups
        .as_object()
        .unwrap_or_else(|| panic!("{label} 不是分组对象: {groups}"));
    assert_eq!(object.len(), 3, "{label} 应只有菜、饭、汤三组: {groups}");
    for kind in ["菜", "饭", "汤"] {
        let items = object
            .get(kind)
            .unwrap_or_else(|| panic!("{label} 缺少 {kind}"))
            .as_array()
            .unwrap();
        for item in items {
            assert_eq!(item["kind"].as_str(), Some(kind), "{label}: {item}");
        }
    }
}

fn assert_no_internal_terms(text: &str, label: &str) {
    for word in ["倾向", "权重", "Elo", "elo", "preference"] {
        assert!(!text.contains(word), "{label} 出现了 {word}");
    }
}

fn meal<'a>(body: &'a Value, date: &str, slot: &str) -> Option<&'a Value> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["date"] == date && item["slot"] == slot)
}

async fn register_user(api: &Api, super_token: &str, username: &str) -> (String, Uuid) {
    let created = must(
        api.call(
            "POST",
            "/api/admin/invites",
            Some(super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "invite",
    )
    .json();
    let code = created["code"].as_str().unwrap().to_string();
    let listed = must(
        api.call(
            "GET",
            "/api/admin/invites",
            Some(super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "invite list",
    )
    .json();
    assert!(
        !listed.to_string().contains(&code),
        "邀请码列表不能再次出现明文"
    );
    assert!(listed["items"][0].get("code").is_none());
    let ua = format!("ua-{username}");
    // 注册成功即登录：响应直接带会话 cookie，不用再登一次。
    let registered = must(
        api.call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": username, "invite_code": code, "password": "password-1"})),
            Some(ua.as_str()),
            None,
        )
        .await,
        StatusCode::CREATED,
        "register",
    );
    assert_eq!(registered.json()["ok"], true, "register body");
    let token = registered.token();
    let account = registered.json();
    must(
        api.call("GET", "/api/me", Some(&token), None, None, None)
            .await,
        StatusCode::OK,
        "register session",
    );
    (token, uid(&account, "account_id"))
}

async fn prepare() -> Api {
    let admin_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres:///postgres?host=/var/run/postgresql".to_string());
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await
        .expect("连接 Postgres");
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = 'duolinzao_test')",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    if !exists {
        sqlx::query("CREATE DATABASE duolinzao_test")
            .execute(&admin)
            .await
            .expect("创建测试库");
    }
    drop(admin);

    let pool =
        PgPoolOptions::new()
            .max_connections(8)
            // 默认本机 peer 套接字；CI 的 runner 里没有套接字，用 TEST_DATABASE_URL 走 TCP。
            .connect(&std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
                "postgres:///duolinzao_test?host=/var/run/postgresql".to_string()
            }))
            .await
            .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query(
        r#"
        TRUNCATE TABLE
            account, invite_code, canteen, role_assignment, canteen_member, canteen_staff,
            device, session, passkey, totp_credential, email_otp_challenge, magic_link, login_approval, login_attempt,
            shift_plan, meal_template, meal_day_lock, meal_order, dish, menu_pick, score,
            comment, preference, notification, llm_channel, audit_log, app_setting
        RESTART IDENTITY CASCADE
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let image_dir = std::env::temp_dir().join("duolinzao-phase1-images");
    let _ = std::fs::remove_dir_all(&image_dir);
    std::fs::create_dir_all(&image_dir).unwrap();
    let raw = example_config().replace("/tmp/duolinzao-images", image_dir.to_str().unwrap());
    let config = AppConfig::parse(&raw).unwrap();
    let mailer = Arc::new(CapturingMailer::new());
    let scripts = ScriptedBackend::failing_then_script();
    let state = AppState::new(
        pool.clone(),
        config,
        Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 10, 10, 2, 0, 0)
                .single()
                .unwrap(),
        )),
        mailer.clone(),
        Arc::new(scripts.clone()),
    )
    .unwrap();
    duolinzao::ops::session::bootstrap(&state).await.unwrap();
    Api {
        app: router(state),
        pool,
        mailer,
        scripts,
    }
}

#[tokio::test]
async fn phase1_acceptance() {
    let _gate = GATE.lock().await;
    let api = prepare().await;

    let health = must(
        api.call("GET", "/api/health", None, None, None, None).await,
        StatusCode::OK,
        "health",
    );
    assert_eq!(health.json()["name"], "多邻灶");
    let ver = health.json()["version"].as_str().unwrap_or("").to_string();
    assert!(!ver.is_empty(), "health 应带 version");
    assert!(
        ver == env!("CARGO_PKG_VERSION") || ver.contains('+') || ver.contains('.'),
        "version 格式异常：{ver}"
    );

    let methods = must(
        api.call("GET", "/api/session/methods", None, None, None, None)
            .await,
        StatusCode::OK,
        "methods",
    )
    .json();
    assert_eq!(methods["methods"], json!(["passkey", "totp", "password"]));
    assert_eq!(methods["email_enabled"], false);

    let otp_off = must(
        api.call(
            "POST",
            "/api/session/email-otp/request",
            None,
            Some(json!({"username": "root"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "otp off",
    );
    assert_eq!(otp_off.json()["error"], "email_disabled");

    let push = must(
        api.call(
            "POST",
            "/api/push/reservations",
            None,
            Some(json!({})),
            None,
            None,
        )
        .await,
        StatusCode::NOT_IMPLEMENTED,
        "push",
    );
    assert_eq!(push.json()["error"], "push_not_in_phase1");

    let super_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("phase1-root"),
            None,
        )
        .await,
        StatusCode::OK,
        "super login",
    )
    .token();
    let super_again = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("phase1-root-2"),
            None,
        )
        .await,
        StatusCode::OK,
        "super second login",
    )
    .token();
    assert_ne!(super_token, super_again);
    must(
        api.call("GET", "/api/me", Some(&super_token), None, None, None)
            .await,
        StatusCode::OK,
        "session 1",
    );
    must(
        api.call("GET", "/api/me", Some(&super_again), None, None, None)
            .await,
        StatusCode::OK,
        "session 2",
    );

    let canteen_a = uid(
        &must(
            api.call(
                "POST",
                "/api/admin/canteens",
                Some(&super_token),
                Some(json!({"name": "一食堂"})),
                None,
                None,
            )
            .await,
            StatusCode::CREATED,
            "canteen a",
        )
        .json(),
        "id",
    );
    let canteen_b = uid(
        &must(
            api.call(
                "POST",
                "/api/admin/canteens",
                Some(&super_token),
                Some(json!({"name": "二食堂"})),
                None,
                None,
            )
            .await,
            StatusCode::CREATED,
            "canteen b",
        )
        .json(),
        "id",
    );

    let bad = must(
        api.call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "nobody", "invite_code": "nope", "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "bad invite",
    );
    assert_eq!(bad.json()["error"], "invite_invalid");

    let (emp1, emp1_id) = register_user(&api, &super_token, "emp-a").await;
    let (emp2, emp2_id) = register_user(&api, &super_token, "emp-b").await;
    let (_cook, cook_id) = register_user(&api, &super_token, "cook-a").await;
    let (_boss, boss_id) = register_user(&api, &super_token, "boss-a").await;

    must(
        api.call(
            "POST",
            "/api/admin/role-assignments",
            Some(&super_token),
            Some(json!({"account_id": cook_id, "role": "canteen", "canteen_ids": [canteen_a]})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "assign cook",
    );
    must(
        api.call(
            "POST",
            "/api/admin/role-assignments",
            Some(&super_token),
            Some(json!({"account_id": boss_id, "role": "admin", "canteen_ids": [canteen_a]})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "assign boss",
    );
    let cook = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "cook-a", "password": "password-1"})),
            Some("ua-cook"),
            None,
        )
        .await,
        StatusCode::OK,
        "cook relogin",
    )
    .token();
    let boss = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "boss-a", "password": "password-1"})),
            Some("ua-boss"),
            None,
        )
        .await,
        StatusCode::OK,
        "boss relogin",
    )
    .token();

    must(
        api.call("GET", "/api/admin/audit", Some(&cook), None, None, None)
            .await,
        StatusCode::FORBIDDEN,
        "cook cannot audit",
    );
    must(
        api.call("POST", "/api/admin/exports", Some(&boss), None, None, None)
            .await,
        StatusCode::FORBIDDEN,
        "admin cannot export",
    );
    must(
        api.call("POST", "/api/admin/invites", Some(&cook), None, None, None)
            .await,
        StatusCode::FORBIDDEN,
        "canteen cannot invite",
    );

    for (token, ids) in [(&emp1, vec![canteen_a]), (&emp2, vec![canteen_a])] {
        must(
            api.call(
                "PUT",
                "/api/me/canteens",
                Some(token),
                Some(json!({"canteen_ids": ids})),
                None,
                None,
            )
            .await,
            StatusCode::OK,
            "bind canteen",
        );
    }

    let dish_path = format!("/api/canteens/{canteen_a}/dishes");
    let veg = must(
        api.call(
            "POST",
            &dish_path,
            Some(&cook),
            Some(json!({"name": "宫保鸡丁", "kind": "菜", "taste_tags": ["微辣"]})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "create 菜",
    )
    .json();
    assert!(veg["image_url"].is_null(), "不上传图片仍可保存");
    let veg_id = uid(&veg, "id");
    let rice = must(
        api.call(
            "POST",
            &dish_path,
            Some(&cook),
            Some(json!({"name": "白米饭", "kind": "饭"})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "create 饭",
    )
    .json();
    let rice_id = uid(&rice, "id");
    let soup = must(
        api.call(
            "POST",
            &dish_path,
            Some(&cook),
            Some(json!({"name": "番茄蛋汤", "kind": "汤"})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "create 汤",
    )
    .json();
    let soup_id = uid(&soup, "id");

    let bad_kind = must(
        api.call(
            "POST",
            &dish_path,
            Some(&cook),
            Some(json!({"name": "点心", "kind": "点心"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "bad kind",
    );
    assert_eq!(bad_kind.json()["error"], "kind_invalid");
    let missing_kind = api
        .call(
            "POST",
            &dish_path,
            Some(&cook),
            Some(json!({"name": "没有类别"})),
            None,
            None,
        )
        .await;
    assert!(
        missing_kind.status == StatusCode::UNPROCESSABLE_ENTITY
            || missing_kind.status == StatusCode::BAD_REQUEST,
        "缺少类别应拒绝: {}",
        missing_kind.json()
    );
    let foreign = must(
        api.call(
            "POST",
            &format!("/api/canteens/{canteen_b}/dishes"),
            Some(&cook),
            Some(json!({"name": "越权菜", "kind": "菜"})),
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "canteen scope",
    );
    assert_eq!(foreign.json()["error"], "forbidden");

    let invalid_image = must(
        api.upload(
            &format!("/api/canteens/{canteen_a}/dishes/{veg_id}/image"),
            &cook,
            b"not-an-image",
        )
        .await,
        StatusCode::BAD_REQUEST,
        "bad image",
    );
    assert_eq!(invalid_image.json()["error"], "image_invalid");
    let oversize = vec![0_u8; 1_048_577];
    let too_big = must(
        api.upload(
            &format!("/api/canteens/{canteen_a}/dishes/{veg_id}/image"),
            &cook,
            &oversize,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "oversize image",
    );
    assert_eq!(too_big.json()["error"], "image_too_large");
    let listed = must(
        api.call("GET", &dish_path, Some(&cook), None, None, None)
            .await,
        StatusCode::OK,
        "list dishes",
    )
    .json();
    assert_groups(&listed["groups"], "菜谱");
    assert!(listed["groups"]["菜"][0]["image_url"].is_null());
    must(
        api.upload(
            &format!("/api/canteens/{canteen_a}/dishes/{veg_id}/image"),
            &cook,
            PNG,
        )
        .await,
        StatusCode::OK,
        "upload png",
    );
    let only_rice = must(
        api.call(
            "GET",
            &format!("{dish_path}?kind=饭"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "filter 饭",
    )
    .json();
    assert_groups(&only_rice["groups"], "筛选饭");
    assert_eq!(only_rice["groups"]["饭"].as_array().unwrap().len(), 1);
    assert!(only_rice["groups"]["菜"].as_array().unwrap().is_empty());
    assert!(only_rice["groups"]["汤"].as_array().unwrap().is_empty());

    let blocked = must(
        api.call(
            "GET",
            &format!("/api/dishes/{veg_id}/image"),
            Some(&emp2),
            None,
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "employee image before order",
    );
    assert_eq!(blocked.json()["error"], "forbidden");

    let week = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/selection/week?date=2026-10-10"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "week",
    )
    .json();
    assert_eq!(week["week_start"], "2026-10-05");
    let days = week["days"].as_array().unwrap();
    assert_eq!(days.len(), 7);
    assert_eq!(days[0]["date"], "2026-10-05");
    assert_eq!(days[6]["date"], "2026-10-11");
    for day in days {
        for slot in ["breakfast", "lunch", "dinner"] {
            assert_groups(&day["slots"][slot]["groups"], "周视图每餐");
        }
    }
    assert_no_internal_terms(&week.to_string(), "周视图");

    let picked = json!({"dish_ids": [veg_id, rice_id, soup_id]});
    must(
        api.call(
            "PUT",
            &format!("/api/canteens/{canteen_a}/selection/2026-10-10/breakfast"),
            Some(&cook),
            Some(picked),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "pick breakfast",
    );
    must(
        api.call(
            "PUT",
            &format!("/api/canteens/{canteen_a}/selection/2026-10-10/lunch"),
            Some(&cook),
            Some(json!({"dish_ids": [veg_id]})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "pick lunch",
    );
    let breakfast = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/selection?date=2026-10-10&slot=breakfast"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "selection breakfast",
    )
    .json();
    assert_groups(&breakfast["groups"], "当餐");
    assert_eq!(
        breakfast["next"],
        json!({"date": "2026-10-10", "slot": "lunch"})
    );
    assert!(breakfast["groups"]["菜"][0]["selected"].as_bool().unwrap());
    assert!(breakfast["groups"]["菜"][0]["image_url"].is_string());
    assert_no_internal_terms(&breakfast.to_string(), "当餐选定");
    assert!(!breakfast.to_string().contains("\"mean\""));

    let lunch = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/selection?date=2026-10-10&slot=lunch"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "selection lunch",
    )
    .json();
    assert_eq!(lunch["next"]["slot"], "dinner");
    let dinner = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/selection?date=2026-10-11&slot=dinner"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "sunday dinner",
    )
    .json();
    assert_eq!(
        dinner["next"],
        json!({"date": "2026-10-12", "slot": "breakfast"})
    );

    let again = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/selection?date=2026-10-10&slot=breakfast"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "selection persists",
    )
    .json();
    assert!(again["groups"]["饭"][0]["selected"].as_bool().unwrap());
    assert!(again["groups"]["汤"][0]["selected"].as_bool().unwrap());

    let locked = must(
        api.call(
            "PUT",
            "/api/meals/2026-10-10/breakfast",
            Some(&emp1),
            Some(json!({"ordering": true, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "employee locked",
    );
    assert_eq!(locked.json()["error"], "meal_locked");
    must(
        api.call(
            "PUT",
            &format!("/api/admin/meals/{emp1_id}/2026-10-10/breakfast"),
            Some(&emp1),
            Some(json!({"ordering": true, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "employee cannot override",
    );
    must(
        api.call(
            "PUT",
            &format!("/api/admin/meals/{emp1_id}/2026-10-10/breakfast"),
            Some(&boss),
            Some(json!({"ordering": true, "canteen_id": canteen_b})),
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "admin other canteen",
    );
    must(
        api.call(
            "PUT",
            &format!("/api/admin/meals/{emp1_id}/2026-10-10/breakfast"),
            Some(&boss),
            Some(json!({"ordering": true, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "admin override locked meal",
    );
    must(
        api.call(
            "PUT",
            &format!("/api/admin/meals/{emp2_id}/2026-10-10/breakfast"),
            Some(&boss),
            Some(json!({"ordering": true, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "override emp2",
    );
    must(
        api.call(
            "PUT",
            &format!("/api/admin/meals/{emp1_id}/2026-10-10/lunch"),
            Some(&boss),
            Some(json!({"ordering": true, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "override lunch for window test",
    );

    let image = api
        .call(
            "GET",
            &format!("/api/dishes/{veg_id}/image"),
            Some(&emp1),
            None,
            None,
            None,
        )
        .await;
    assert_eq!(image.status, StatusCode::OK, "{}", image.json());
    assert!(image.bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]));

    let closed = must(
        api.call(
            "POST",
            "/api/scores",
            Some(&emp1),
            Some(
                json!({"meal_date": "2026-10-10", "slot": "lunch", "dish_id": veg_id, "value": -1}),
            ),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "score window",
    );
    assert_eq!(closed.json()["error"], "score_window_closed");
    let comment_only = must(
        api.call(
            "POST",
            "/api/scores",
            Some(&emp1),
            Some(json!({
                "meal_date": "2026-10-10",
                "slot": "breakfast",
                "dish_id": veg_id,
                "comment": "只有评语"
            })),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "comment only",
    );
    assert_eq!(comment_only.json()["error"], "comment_only");

    let targets = must(
        api.call(
            "GET",
            "/api/me/score-targets?date=2026-10-10&slot=breakfast",
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "score targets",
    )
    .json();
    assert_eq!(targets["window_open"], true);
    assert_groups(&targets["groups"], "可评列表");
    assert!(targets["groups"]["菜"][0]["image_url"].is_string());
    assert!(targets["groups"]["饭"][0]["image_url"].is_null());
    assert_no_internal_terms(&targets.to_string(), "员工可评");

    must(
        api.call(
            "POST",
            "/api/scores",
            Some(&emp1),
            Some(json!({
                "meal_date": "2026-10-10",
                "slot": "breakfast",
                "dish_id": veg_id,
                "value": -2,
                "comment": "偏咸",
                "anonymous": true
            })),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "score emp1",
    );
    let duplicate = must(
        api.call(
            "POST",
            "/api/scores",
            Some(&emp1),
            Some(json!({
                "meal_date": "2026-10-10",
                "slot": "breakfast",
                "dish_id": veg_id,
                "value": 1
            })),
            None,
            None,
        )
        .await,
        StatusCode::CONFLICT,
        "duplicate score",
    );
    assert_eq!(duplicate.json()["error"], "score_exists");
    must(
        api.call(
            "POST",
            "/api/scores",
            Some(&emp2),
            Some(json!({
                "meal_date": "2026-10-10",
                "slot": "breakfast",
                "dish_id": veg_id,
                "value": -1,
                "comment": "还行",
                "anonymous": false
            })),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "score emp2",
    );

    let comments = must(
        api.call(
            "GET",
            &format!("/api/admin/comments?canteen_id={canteen_a}"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "comments",
    )
    .json();
    assert_groups(&comments["groups"], "评语");
    let texts = comments["groups"]["菜"].as_array().unwrap();
    let anon = texts.iter().find(|item| item["anonymous"] == true).unwrap();
    assert!(anon.get("username").unwrap().is_null());
    assert!(anon.get("display_name").unwrap().is_null());
    let named = texts
        .iter()
        .find(|item| item["anonymous"] == false)
        .unwrap();
    assert_eq!(named["username"], "emp-b");
    assert!(named.get("display_name").unwrap().is_null());
    must(
        api.call(
            "GET",
            &format!("/api/admin/comments?canteen_id={canteen_b}"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "comments out of scope",
    );

    let alerts = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/alerts"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "alerts",
    )
    .json();
    assert_groups(&alerts["groups"], "强提醒");
    assert_eq!(alerts["groups"]["菜"].as_array().unwrap().len(), 1);
    assert_eq!(alerts["groups"]["菜"][0]["enabled"], true);
    assert_eq!(
        alerts["groups"]["菜"][0]["message"],
        "样本已足够，均分低于当前阈值"
    );
    let notes = must(
        api.call("GET", "/api/notifications", Some(&cook), None, None, None)
            .await,
        StatusCode::OK,
        "notifications",
    )
    .json();
    assert!(notes["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["body"] == "样本已足够，均分低于当前阈值"));
    let still = must(
        api.call("GET", &dish_path, Some(&cook), None, None, None)
            .await,
        StatusCode::OK,
        "dish still enabled",
    )
    .json();
    assert_eq!(still["groups"]["菜"][0]["enabled"], true);
    must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/alerts"),
            Some(&boss),
            None,
            None,
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
        "admin is not canteen alert audience",
    );

    let summaries = must(
        api.call(
            "GET",
            &format!("/api/admin/score-summaries?canteen_id={canteen_a}"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "summaries",
    )
    .json();
    assert_groups(&summaries["groups"], "评分汇总");
    assert_no_internal_terms(&summaries.to_string(), "评分汇总");
    let colored = must(
        api.call(
            "GET",
            &format!("/api/canteens/{canteen_a}/selection?date=2026-10-10&slot=breakfast"),
            Some(&cook),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "tone",
    )
    .json();
    assert_eq!(colored["groups"]["菜"][0]["tone"], "red");
    assert!(!colored.to_string().contains("\"mean\""));
    assert_no_internal_terms(&colored.to_string(), "色阶");

    let open_meal = must(
        api.call(
            "PUT",
            "/api/meals/2026-10-11/lunch",
            Some(&emp1),
            Some(json!({"ordering": true, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "manual open meal",
    )
    .json();
    assert_eq!(open_meal["color"], "green");
    let template = must(
        api.call(
            "GET",
            "/api/me/meal-template",
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "template after manual",
    )
    .json();
    assert!(template["dsl"]["overrides"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["date"] == "2026-10-11"
            && item["slot"] == "lunch"
            && item["ordering"] == true));

    let shifted = must(
        api.call(
            "PUT",
            "/api/me/shift",
            Some(&emp1),
            Some(json!({"kind": "work2_rest4", "anchor_date": "2026-10-12", "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "shift",
    )
    .json();
    assert_eq!(shifted["dsl"]["mode"], "work2_rest4");
    let template_id = uid(&shifted, "id");
    let meals = must(
        api.call(
            "GET",
            "/api/me/meals?start=2026-10-10&end=2026-10-16",
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "meals after shift",
    )
    .json();
    assert_eq!(
        meal(&meals, "2026-10-10", "breakfast").unwrap()["ordering"],
        true
    );
    assert_eq!(
        meal(&meals, "2026-10-11", "lunch").unwrap()["ordering"],
        false
    );
    assert!(
        meal(&meals, "2026-10-14", "lunch").is_none(),
        "休息日且没有旧记录时不自动报餐"
    );
    assert_eq!(
        meal(&meals, "2026-10-12", "lunch").unwrap()["ordering"],
        true
    );

    must(
        api.call(
            "PUT",
            "/api/meals/2026-10-12/lunch",
            Some(&emp1),
            Some(json!({"ordering": false, "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "clear unlocked lunch",
    );
    must(
        api.call(
            "PUT",
            "/api/me/shift",
            Some(&emp1),
            Some(json!({"kind": "work2_rest4", "anchor_date": "2026-10-12", "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "shift overwrites unlocked",
    );
    let meals = must(
        api.call(
            "GET",
            "/api/me/meals?start=2026-10-10&end=2026-10-16",
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "meals after regenerate",
    )
    .json();
    assert_eq!(
        meal(&meals, "2026-10-12", "lunch").unwrap()["ordering"],
        true
    );
    assert_eq!(
        meal(&meals, "2026-10-10", "breakfast").unwrap()["ordering"],
        true
    );
    let locked_dates = meals["locked_dates"].as_array().unwrap();
    assert!(locked_dates.iter().any(|date| date == "2026-10-10"));
    assert!(!locked_dates.iter().any(|date| date == "2026-10-11"));
    let home = must(
        api.call("GET", "/api/me/home", Some(&emp1), None, None, None)
            .await,
        StatusCode::OK,
        "employee home",
    )
    .json();
    assert_eq!(home["days"].as_array().unwrap().len(), 7);
    assert_eq!(home["days"][0]["date"], "2026-10-10");
    assert_eq!(home["days"][0]["locked"], true);
    assert_eq!(home["days"][0]["slots"]["dinner"]["state"], "locked");
    assert_eq!(home["days"][1]["date"], "2026-10-11");
    assert_eq!(home["days"][1]["locked"], false);
    assert_eq!(home["next"]["date"], "2026-10-10");
    assert_eq!(home["next"]["slot"], "lunch");
    assert_eq!(home["next"]["canteen_name"], "一食堂");
    assert_eq!(home["next"]["menu"]["菜"], json!(["宫保鸡丁"]));
    assert_eq!(home["next"]["menu"]["饭"], json!([]));
    assert_eq!(home["next"]["menu"]["汤"], json!([]));
    let home_text = home.to_string();
    assert!(!home_text.contains("\"mean\""));
    assert!(!home_text.contains("preference"));

    let five = must(
        api.call(
            "PUT",
            "/api/me/shift",
            Some(&emp1),
            Some(json!({"kind": "上五休二"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "no 上五休二",
    );
    assert_eq!(five.json()["error"], "shift_kind_invalid");

    let edited = must(
        api.call(
            "PUT",
            &format!("/api/meal-templates/{template_id}/slots"),
            Some(&emp1),
            Some(json!({
                "date": "2026-10-15",
                "slot": "dinner",
                "ordering": true,
                "canteen_id": canteen_a
            })),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "light edit",
    )
    .json();
    assert_eq!(edited["dsl_updated"], true);
    assert_eq!(edited["order_updated"], true);
    assert_eq!(edited["color"], "green");
    let locked_edit = must(
        api.call(
            "PUT",
            &format!("/api/meal-templates/{template_id}/slots"),
            Some(&emp1),
            Some(json!({
                "date": "2026-10-10",
                "slot": "dinner",
                "ordering": false,
                "canteen_id": canteen_a
            })),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "locked dsl edit",
    )
    .json();
    assert_eq!(locked_edit["dsl_updated"], true);
    assert_eq!(locked_edit["order_updated"], false);
    assert_eq!(locked_edit["locked"], true);
    let meals = must(
        api.call(
            "GET",
            "/api/me/meals?start=2026-10-10&end=2026-10-16",
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "locked meal unchanged",
    )
    .json();
    assert_eq!(
        meal(&meals, "2026-10-10", "breakfast").unwrap()["ordering"],
        true
    );
    assert!(meal(&meals, "2026-10-10", "dinner").is_none());
    assert_eq!(
        meal(&meals, "2026-10-15", "dinner").unwrap()["ordering"],
        true
    );

    api.scripts.calls.lock().unwrap().clear();
    let secret = "sk-phase1-secret-value";
    must(
        api.call(
            "PUT",
            "/api/admin/llm-channels",
            Some(&super_token),
            Some(json!({
                "channels": [
                    {"kind": "anthropic", "api_key": "unused", "enabled": false, "model": "claude"},
                    {"kind": "openai_chat", "api_key": secret, "enabled": true, "base_url": "https://example.invalid/v1", "model": "demo"},
                    {"kind": "local_script", "enabled": true}
                ]
            })),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "channels",
    );
    let channels = must(
        api.call(
            "GET",
            "/api/admin/llm-channels",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "get channels",
    )
    .json();
    assert!(!channels.to_string().contains(secret));
    assert_eq!(channels["channels"][1]["kind"], "openai_chat");
    assert_eq!(channels["channels"][1]["key_configured"], true);
    let rejected = must(
        api.call(
            "POST",
            "/api/meal-templates/generate",
            Some(&emp1),
            Some(json!({"description": "我想上五休二", "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "generate rejects 上五休二",
    );
    assert_eq!(rejected.json()["error"], "shift_kind_invalid");
    assert!(api.scripts.calls.lock().unwrap().is_empty());
    let generated = must(
        api.call(
            "POST",
            "/api/meal-templates/generate",
            Some(&emp1),
            Some(json!({
                "description": "上二休四",
                "anchor_date": "2026-10-12",
                "canteen_id": canteen_a
            })),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "generate",
    )
    .json();
    assert_eq!(generated["dsl"]["mode"], "work2_rest4");
    assert_eq!(
        generated["preview"][0]["slots"]["breakfast"]["locked"],
        true
    );
    assert_eq!(
        generated["preview"][0]["slots"]["breakfast"]["color"],
        "locked"
    );
    assert_eq!(
        generated["preview"][1]["slots"]["breakfast"]["locked"],
        false
    );
    assert_eq!(
        api.scripts.calls.lock().unwrap().as_slice(),
        ["openai_chat".to_string(), "local_script".to_string()]
    );
    must(
        api.call(
            "PUT",
            "/api/admin/llm-channels",
            Some(&super_token),
            Some(json!({
                "channels": [
                    {"kind": "openai_chat", "api_key": secret, "enabled": true, "base_url": "https://example.invalid/v1", "model": "demo"}
                ]
            })),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "remote only",
    );
    *api.scripts.fail_kinds.lock().unwrap() = Vec::new();
    *api.scripts.response.lock().unwrap() = Some(
        "```json\n{\"mode\":\"custom\",\"week\":[{\"weekday\":1,\"breakfast\":false,\"lunch\":true,\"dinner\":false}]}\n```"
            .into(),
    );
    let fenced = must(
        api.call(
            "POST",
            "/api/meal-templates/generate",
            Some(&emp1),
            Some(json!({"description": "只在周一吃午饭", "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "generate from fenced json",
    )
    .json();
    assert_eq!(fenced["dsl"]["mode"], "custom");
    assert_eq!(fenced["dsl"]["week"][0]["lunch"], true);
    *api.scripts.response.lock().unwrap() = Some("这不是班制".into());
    let unparsed = must(
        api.call(
            "POST",
            "/api/meal-templates/generate",
            Some(&emp1),
            Some(json!({"description": "随便写一段", "canteen_id": canteen_a})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "parse failure",
    )
    .json();
    assert_eq!(unparsed["error"], "dsl_parse");
    assert_eq!(unparsed["message"], "解析失败");
    *api.scripts.fail_kinds.lock().unwrap() = vec![
        "openai_chat".into(),
        "openai_responses".into(),
        "anthropic".into(),
    ];
    *api.scripts.response.lock().unwrap() = None;

    let email_setting = must(
        api.call(
            "GET",
            "/api/admin/settings/email",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "email setting",
    )
    .json();
    assert_eq!(email_setting["enabled"], false);
    assert_eq!(email_setting["smtp_configured"], true);
    assert!(!email_setting.to_string().contains("127.0.0.1"));
    assert!(!email_setting.to_string().contains("noreply"));
    must(
        api.call(
            "PUT",
            "/api/admin/settings/email",
            Some(&super_token),
            Some(json!({"enabled": true})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "enable email",
    );
    let methods = must(
        api.call("GET", "/api/session/methods", None, None, None, None)
            .await,
        StatusCode::OK,
        "methods with email",
    )
    .json();
    assert_eq!(
        methods["methods"],
        json!(["passkey", "totp", "email_otp", "password"])
    );
    must(
        api.call(
            "PUT",
            "/api/me/email",
            Some(&emp1),
            Some(json!({"email": "emp-a@example.com"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "bind email",
    );
    // 邮箱 OTP：request 防枚举一律 accepted；未知账号不发信。
    let mails_before = api.mailer.messages.lock().unwrap().len();
    let ghost = must(
        api.call(
            "POST",
            "/api/session/email-otp/request",
            None,
            Some(json!({"username": "ghost-nobody"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "otp request unknown",
    )
    .json();
    assert_eq!(ghost["accepted"], true);
    assert_eq!(
        api.mailer.messages.lock().unwrap().len(),
        mails_before,
        "未知账号不发信"
    );
    let accepted = must(
        api.call(
            "POST",
            "/api/session/email-otp/request",
            None,
            Some(json!({"username": "emp-a"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "otp request",
    )
    .json();
    assert_eq!(accepted["accepted"], true);
    let mail = api.mailer.messages.lock().unwrap().clone();
    assert_eq!(mail.len(), mails_before + 1);
    assert!(
        !mail[mails_before].2.contains("http"),
        "邮箱 OTP 正文不含登录链接"
    );
    let code = otp_code(&mail[mails_before].2);
    // 未到 60 秒重发拒绝。
    let soon = api
        .call(
            "POST",
            "/api/session/email-otp/request",
            None,
            Some(json!({"username": "emp-a"})),
            None,
            None,
        )
        .await;
    assert_eq!(soon.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(soon.json()["error"], "otp_resend_too_soon");
    let otp_token = must(
        api.call(
            "POST",
            "/api/session/email-otp/verify",
            None,
            Some(json!({"username": "emp-a", "code": code.clone()})),
            Some("otp-ua"),
            None,
        )
        .await,
        StatusCode::OK,
        "otp verify",
    )
    .token();
    must(
        api.call("GET", "/api/me", Some(&otp_token), None, None, None)
            .await,
        StatusCode::OK,
        "otp session",
    );
    must(
        api.call("GET", "/api/me", Some(&emp1), None, None, None)
            .await,
        StatusCode::OK,
        "old session after otp",
    );
    // 一次性：同一码再验拒绝。
    let reused = api
        .call(
            "POST",
            "/api/session/email-otp/verify",
            None,
            Some(json!({"username": "emp-a", "code": code})),
            None,
            None,
        )
        .await;
    assert_eq!(reused.status, StatusCode::BAD_REQUEST);
    assert_eq!(reused.json()["error"], "otp_reused");
    // 旧 magic-link 路由已移除，不可用。
    for dead in [
        "/api/session/magic-link/request",
        "/api/session/magic-link/consume",
    ] {
        let res = api
            .call(
                "POST",
                dead,
                None,
                Some(json!({"token": "x"})),
                None,
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{dead} 应不可用");
    }
    // 失败锁定：连续 5 次验错锁 15 分钟，正确码也拒绝（独立账号/IP，避开共用计数窗）。
    let lock_ip: SocketAddr = "198.51.100.9:443".parse().unwrap();
    let (otp_user, _) = register_user(&api, &super_token, "emp-otp").await;
    must(
        api.call(
            "PUT",
            "/api/me/email",
            Some(&otp_user),
            Some(json!({"email": "emp-otp@example.com"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "bind otp email",
    );
    must(
        api.call(
            "POST",
            "/api/session/email-otp/request",
            None,
            Some(json!({"username": "emp-otp"})),
            None,
            Some(lock_ip),
        )
        .await,
        StatusCode::OK,
        "otp request for lock",
    );
    let mail = api.mailer.messages.lock().unwrap().clone();
    let good = otp_code(&mail.last().unwrap().2);
    for round in 0..5 {
        let bad = api
            .call(
                "POST",
                "/api/session/email-otp/verify",
                None,
                Some(json!({"username": "emp-otp", "code": "000000"})),
                None,
                Some(lock_ip),
            )
            .await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST, "第 {round} 次验错");
        assert_eq!(bad.json()["error"], "otp_invalid");
    }
    let locked = api
        .call(
            "POST",
            "/api/session/email-otp/verify",
            None,
            Some(json!({"username": "emp-otp", "code": good})),
            None,
            Some(lock_ip),
        )
        .await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(locked.json()["error"], "otp_locked");

    let setup = must(
        api.call("POST", "/api/me/totp/setup", Some(&emp2), None, None, None)
            .await,
        StatusCode::OK,
        "totp setup",
    )
    .json();
    let secret_b32 = setup["secret"].as_str().unwrap();
    let secret_bytes = totp_rs::Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .unwrap();
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some("多邻灶".into()),
        "emp-b".into(),
    )
    .unwrap();
    let now = Utc
        .with_ymd_and_hms(2026, 10, 10, 2, 0, 0)
        .unwrap()
        .timestamp() as u64;
    let code = totp.generate(now);
    must(
        api.call(
            "POST",
            "/api/me/totp/confirm",
            Some(&emp2),
            Some(json!({"code": code})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "totp confirm",
    );
    // 认证器 TOTP 防重放：确认用过的码在同一时间窗内登录拒绝。
    let replay = api
        .call(
            "POST",
            "/api/session/totp",
            None,
            Some(json!({"username": "emp-b", "code": code.clone()})),
            Some("totp-ua"),
            None,
        )
        .await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
    assert_eq!(replay.json()["error"], "totp_reused");
    // 把已用窗拨旧后，同一码可登录（证明拦的是重放，不是码本身）。
    sqlx::query("UPDATE totp_credential SET last_used_step = last_used_step - 2 WHERE account_id = $1")
        .bind(emp2_id)
        .execute(&api.pool)
        .await
        .unwrap();
    let totp_token = must(
        api.call(
            "POST",
            "/api/session/totp",
            None,
            Some(json!({"username": "emp-b", "code": code})),
            Some("totp-ua"),
            None,
        )
        .await,
        StatusCode::OK,
        "totp login",
    )
    .token();
    must(
        api.call("GET", "/api/me", Some(&totp_token), None, None, None)
            .await,
        StatusCode::OK,
        "totp session",
    );
    must(
        api.call("GET", "/api/me", Some(&emp2), None, None, None)
            .await,
        StatusCode::OK,
        "password session remains",
    );

    let ip: SocketAddr = "203.0.113.8:443".parse().unwrap();
    let approval = must(
        api.call(
            "POST",
            "/api/login-approvals",
            None,
            Some(json!({"username": "emp-a"})),
            Some(EDGE_UA),
            Some(ip),
        )
        .await,
        StatusCode::CREATED,
        "approval create",
    )
    .json();
    assert_eq!(approval["ip"], "203.0.113.8");
    assert_eq!(approval["friendly_name"], "Windows · Edge");
    let approval_id = approval["id"].as_str().unwrap().to_string();
    let poll_token = approval["poll_token"].as_str().unwrap().to_string();
    let pending = must(
        api.call(
            "GET",
            "/api/login-approvals/pending",
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "pending",
    )
    .json();
    assert_eq!(pending["items"][0]["user_agent"], EDGE_UA);
    assert_eq!(pending["items"][0]["ip"], "203.0.113.8");
    must(
        api.call(
            "POST",
            &format!("/api/login-approvals/{approval_id}/allow"),
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "allow",
    );
    let polled = must(
        api.call(
            "GET",
            &format!("/api/login-approvals/{approval_id}?poll_token={poll_token}"),
            None,
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "poll allow",
    );
    assert_eq!(polled.json()["status"], "allowed");
    let approved = polled.token();
    must(
        api.call("GET", "/api/me", Some(&approved), None, None, None)
            .await,
        StatusCode::OK,
        "approved session",
    );
    must(
        api.call("GET", "/api/me", Some(&emp1), None, None, None)
            .await,
        StatusCode::OK,
        "original still in",
    );
    let passkey = must(
        api.call(
            "POST",
            "/api/passkeys/register/start",
            Some(&approved),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "passkey register start",
    )
    .json();
    assert!(passkey["challenge_id"].is_string());

    let denied = must(
        api.call(
            "POST",
            "/api/login-approvals",
            None,
            Some(json!({"username": "emp-a"})),
            Some("deny-ua"),
            None,
        )
        .await,
        StatusCode::CREATED,
        "second approval",
    )
    .json();
    let denied_id = denied["id"].as_str().unwrap();
    let denied_poll = denied["poll_token"].as_str().unwrap();
    must(
        api.call(
            "POST",
            &format!("/api/login-approvals/{denied_id}/deny"),
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "deny",
    );
    let denied_poll_res = must(
        api.call(
            "GET",
            &format!("/api/login-approvals/{denied_id}?poll_token={denied_poll}"),
            None,
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "poll deny",
    );
    assert_eq!(denied_poll_res.json()["status"], "denied");
    assert!(denied_poll_res.cookie.is_none());

    let devices = must(
        api.call("GET", "/api/devices", Some(&emp1), None, None, None)
            .await,
        StatusCode::OK,
        "devices",
    )
    .json();
    let edge = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["friendly_name"] == "Windows · Edge")
        .unwrap();
    let edge_id = edge["id"].as_str().unwrap();
    must(
        api.call(
            "DELETE",
            &format!("/api/devices/{edge_id}"),
            Some(&emp1),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "kick",
    );
    must(
        api.call("GET", "/api/me", Some(&approved), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "kicked session",
    );
    must(
        api.call("GET", "/api/me", Some(&emp1), None, None, None)
            .await,
        StatusCode::OK,
        "unkicked session",
    );

    let audit = must(
        api.call(
            "GET",
            "/api/admin/audit",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "audit",
    )
    .json();
    let registered = audit["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["action"] == "register" && item["actor_username"] == "emp-a")
        .expect("注册审计");
    assert_eq!(registered["role"], "employee");
    assert_eq!(registered["scope"], "self");
    assert_eq!(registered["summary"]["auto_login"], true);
    assert!(registered["actor_id"].is_string());
    let young_id = Uuid::parse_str(registered["id"].as_str().unwrap()).unwrap();
    let deleted = sqlx::query("DELETE FROM audit_log WHERE id = $1")
        .bind(young_id)
        .execute(&api.pool)
        .await;
    assert!(deleted.is_err(), "未满 6 个月的审计不能删");
    sqlx::query(
        "INSERT INTO audit_log (created_at, scope, action, result) VALUES (now() - interval '7 months', 'self', 'ancient', 'ok')",
    )
    .execute(&api.pool)
    .await
    .unwrap();
    let purged: i64 = sqlx::query_scalar("SELECT purge_audit_older_than_6_months()")
        .fetch_one(&api.pool)
        .await
        .unwrap();
    assert!(purged >= 1);
    let young_remains: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE id = $1)")
            .bind(young_id)
            .fetch_one(&api.pool)
            .await
            .unwrap();
    assert!(young_remains);
    let ancient: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE action = 'ancient')")
            .fetch_one(&api.pool)
            .await
            .unwrap();
    assert!(!ancient);

    let accounts = must(
        api.call(
            "GET",
            "/api/admin/accounts",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "accounts",
    )
    .json();
    assert!(!accounts.to_string().contains("password_hash"));
    assert!(!accounts.to_string().contains("$argon2"));
    let exported = api
        .call(
            "POST",
            "/api/admin/exports",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await;
    assert_eq!(exported.status, StatusCode::OK, "{}", exported.json());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported.bytes)).unwrap();
    let mut names = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        names.push(file.name().to_string());
        if file.name().ends_with(".json") {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut file, &mut text).unwrap();
            assert!(!text.contains("password_hash"), "{}", file.name());
            assert!(!text.contains(secret), "{}", file.name());
            assert!(!text.contains("$argon2"), "{}", file.name());
        }
    }
    for required in [
        "canteens.json",
        "dishes.json",
        "meal_orders.json",
        "scores.json",
        "comments.json",
        "audit.json",
    ] {
        assert!(names.iter().any(|name| name == required), "{names:?}");
    }
    assert!(names.iter().any(|name| name.starts_with("images/")));

    must(
        api.call(
            "POST",
            "/api/admin/recovery",
            Some(&super_token),
            Some(
                json!({"username": "emp-b", "action": "reset_password", "password": "password-2"}),
            ),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "recover",
    );
    must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "emp-b", "password": "password-2"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "login after recover",
    );
    must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "emp-b", "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "old password rejected",
    );
}

#[tokio::test]
async fn csrf_gate_and_security_headers() {
    let _gate = GATE.lock().await;
    let api = prepare().await;

    // 写操作缺头应拒绝：走无需会话的注册接口做探针。
    let bare = Request::builder()
        .method("POST")
        .uri("/api/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"username": "x", "invite_code": "y", "password": "z"}))
                .unwrap(),
        ))
        .unwrap();
    let response = api.app.clone().oneshot(bare).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "csrf_required");

    // 带头则放行到业务层（邀请码无效，应是业务错误而非 403）。
    let res = api
        .call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "x", "invite_code": "nope", "password": "password-1"})),
            None,
            None,
        )
        .await;
    assert_ne!(
        res.status,
        StatusCode::FORBIDDEN,
        "带头不应被 CSRF 闸拦: {}",
        res.json()
    );

    // 读操作不受影响；响应应带安全头。
    let bare = Request::builder()
        .method("GET")
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let response = api.app.clone().oneshot(bare).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    let csp = headers
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("缺少 CSP");
    assert!(csp.contains("default-src 'self'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'self'"), "{csp}");
    assert!(
        headers.get("referrer-policy").is_some(),
        "缺少 Referrer-Policy"
    );
}

#[tokio::test]
async fn app_static_no_spa_fallback() {
    let _gate = GATE.lock().await;
    let api = prepare().await;

    // 本用例自建带 static_dir 的 state：公共 prepare 不配静态目录，
    // 否则 fallback 的 ServeDir 会把“无路由 POST”的 404 变成 405，干扰旧断言。
    let app_dir = std::env::temp_dir().join("duolinzao-phase1-app");
    let _ = std::fs::remove_dir_all(&app_dir);
    std::fs::create_dir_all(app_dir.join("app")).unwrap();
    std::fs::write(
        app_dir.join("app/version.json"),
        r#"{"versionCode":1,"versionName":"0.1.2","url":"/app/duolinzao.apk","sha256":"abc"}"#,
    )
    .unwrap();
    std::fs::write(app_dir.join("app/duolinzao.apk"), b"fakepk").unwrap();
    let raw_cfg = example_config().replace(
        "[server]",
        &format!("[server]\nstatic_dir = \"{}\"", app_dir.to_str().unwrap()),
    );
    let config = AppConfig::parse(&raw_cfg).unwrap();
    let state = AppState::new(
        api.pool.clone(),
        config,
        Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 10, 10, 2, 0, 0).single().unwrap(),
        )),
        api.mailer.clone(),
        Arc::new(api.scripts.clone()),
    )
    .unwrap();
    let app = router(state);

    async fn raw(app: axum::Router, uri: &str) -> (StatusCode, Value, Vec<(String, String)>, Vec<u8>) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("user-agent", "phase1-test")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.expect("oneshot");
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or("").to_string(),
                )
            })
            .collect::<Vec<_>>();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body, headers, bytes.to_vec())
    }
    let header = |headers: &[(String, String)], name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };

    // version.json：200 + JSON 体 + JSON 类型 + 不缓存。
    let (status, body, headers, _) = raw(app.clone(), "/app/version.json").await;
    assert_eq!(status, StatusCode::OK, "version.json 应直出");
    assert_eq!(body["versionCode"], 1, "{body}");
    assert_eq!(body["url"], "/app/duolinzao.apk", "{body}");
    assert_eq!(header(&headers, "content-type"), "application/json", "{headers:?}");
    assert_eq!(header(&headers, "cache-control"), "no-store", "{headers:?}");

    // APK：200 + Android 包类型。
    let (status, _, headers, bytes) = raw(app.clone(), "/app/duolinzao.apk").await;
    assert_eq!(status, StatusCode::OK, "apk 应直出");
    assert_eq!(bytes, b"fakepk", "应为夹具包内容");
    assert_eq!(
        header(&headers, "content-type"),
        "application/vnd.android.package-archive",
        "{headers:?}"
    );

    // 缺文件：404 JSON（error=not_found），绝不能回退 index.html。
    let (status, body, headers, bytes) = raw(app.clone(), "/app/missing.apk").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "缺文件应 404");
    assert_eq!(body["error"], "not_found", "{body}");
    assert_eq!(header(&headers, "content-type"), "application/json", "{headers:?}");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("<html"),
        "不能回退 SPA 页面"
    );

    // 路径逃逸：同样 404。
    let (status, _, _, _) = raw(app.clone(), "/app/../secret").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "逃逸应 404");
}

#[tokio::test]
async fn app_static_custom_app_dir() {
    let _gate = GATE.lock().await;
    let api = prepare().await;

    // staging 姿态：[server] app_dir 指向另一份目录，/app/* 从它直出，
    // 而 {static_dir}/app 下的文件不再被服务（默认回退只在未配 app_dir 时生效）。
    let base = std::env::temp_dir().join("duolinzao-phase1-appdir");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("static/app")).unwrap();
    std::fs::create_dir_all(base.join("custom")).unwrap();
    std::fs::write(base.join("static/app/version.json"), r#"{"versionCode":1}"#).unwrap();
    std::fs::write(
        base.join("custom/version.json"),
        r#"{"versionCode":7,"versionName":"0.1.5","url":"/app/duolinzao.apk","sha256":"abc"}"#,
    )
    .unwrap();
    let raw_cfg = example_config().replace(
        "[server]",
        &format!(
            "[server]\nstatic_dir = \"{}\"\napp_dir = \"{}\"",
            base.join("static").to_str().unwrap(),
            base.join("custom").to_str().unwrap()
        ),
    );
    let config = AppConfig::parse(&raw_cfg).unwrap();
    let state = AppState::new(
        api.pool.clone(),
        config,
        Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 10, 10, 2, 0, 0).single().unwrap(),
        )),
        api.mailer.clone(),
        Arc::new(api.scripts.clone()),
    )
    .unwrap();
    let app = router(state);
    let request = Request::builder()
        .method("GET")
        .uri("/app/version.json")
        .header("user-agent", "phase1-test")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK, "自定义 app_dir 应直出");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["versionCode"], 7, "应为 custom 目录的包，{body}");

    // 逃逸在自定义目录下同样 404。
    let state2 = AppState::new(
        api.pool.clone(),
        AppConfig::parse(&raw_cfg).unwrap(),
        Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 10, 10, 2, 0, 0).single().unwrap(),
        )),
        api.mailer.clone(),
        Arc::new(api.scripts.clone()),
    )
    .unwrap();
    let app2 = router(state2);
    let request = Request::builder()
        .method("GET")
        .uri("/app/../secret")
        .header("user-agent", "phase1-test")
        .body(Body::empty())
        .unwrap();
    let response = app2.oneshot(request).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::NOT_FOUND, "逃逸应 404");
}

#[tokio::test]
async fn app_signing_keystore_in_db() {
    let _gate = GATE.lock().await;
    let api = prepare().await;
    let super_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("signing-root"),
            None,
        )
        .await,
        StatusCode::OK,
        "super login",
    )
    .token();

    // 未配置：状态为未配置；此时签名发布应明确报未配置（不碰 apksigner）。
    let status = must(
        api.call("GET", "/api/admin/app-signing", Some(&super_token), None, None, None)
            .await,
        StatusCode::OK,
        "signing status",
    )
    .json();
    assert_eq!(status["configured"], false, "{status}");
    let denied = must(
        api.call("POST", "/api/admin/app-signing/sign", Some(&super_token), None, None, None)
            .await,
        StatusCode::BAD_REQUEST,
        "sign without keystore",
    )
    .json();
    assert_eq!(denied["error"], "signing_not_configured", "{denied}");

    // 普通员工：连状态都看不到。
    let (emp_token, _) = register_user(&api, &super_token, "emp-signing").await;
    must(
        api.call("GET", "/api/admin/app-signing", Some(&emp_token), None, None, None)
            .await,
        StatusCode::FORBIDDEN,
        "employee forbidden",
    );

    // 超管上传 keystore：multipart（文件 + 密码/别名）；响应只回显别名与指纹。
    let boundary = "signingboundary";
    let mut payload = Vec::new();
    let mut part = |name: &str, filename: Option<&str>, ctype: &str, data: &[u8]| {
        payload.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"")
                .as_bytes(),
        );
        if let Some(file) = filename {
            payload.extend_from_slice(format!("; filename=\"{file}\"").as_bytes());
        }
        payload.extend_from_slice(format!("\r\nContent-Type: {ctype}\r\n\r\n").as_bytes());
        payload.extend_from_slice(data);
        payload.extend_from_slice(b"\r\n");
    };
    part("keystore", Some("site.jks"), "application/octet-stream", b"fake-jks-bytes");
    part("keystore_password", None, "text/plain", b"ks-pass-1");
    part("key_alias", None, "text/plain", b"test-key");
    part("key_password", None, "text/plain", b"key-pass-1");
    payload.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let request = Request::builder()
        .method("PUT")
        .uri("/api/admin/app-signing")
        .header("x-dlz-requested", "1")
        .header("cookie", format!("dlz_session={super_token}"))
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(payload))
        .unwrap();
    let response = api.app.clone().oneshot(request).await.expect("upload");
    assert_eq!(response.status(), StatusCode::OK, "上传应成功");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let saved: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved["configured"], true, "{saved}");
    assert_eq!(saved["key_alias"], "test-key", "{saved}");
    let fingerprint = saved["keystore_sha256"].as_str().unwrap_or_default().to_string();
    assert_eq!(fingerprint.len(), 64, "指纹应为 sha256 hex，{saved}");
    // 密码与 keystore 本体永不出现在响应里。
    for key in ["keystore", "keystore_password", "key_password", "key_ciphertext"] {
        assert!(saved.get(key).is_none(), "响应不得含 {key}，{saved}");
    }

    // 状态页回显一致，且密码同样不可见。
    let status = must(
        api.call("GET", "/api/admin/app-signing", Some(&super_token), None, None, None)
            .await,
        StatusCode::OK,
        "signing status after put",
    )
    .json();
    assert_eq!(status["configured"], true, "{status}");
    assert_eq!(status["key_alias"], "test-key", "{status}");
    assert_eq!(status["keystore_sha256"], fingerprint.as_str(), "{status}");
}

#[tokio::test]
async fn security_hardening_round2() {
    let _gate = GATE.lock().await;
    let api = prepare().await;
    let super_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("round2-root"),
            None,
        )
        .await,
        StatusCode::OK,
        "super login",
    )
    .token();

    // 1. 改密后旧 cookie 应 401，新密码可登，旧密码不行。
    let (old_token, _) = register_user(&api, &super_token, "emp-revoke").await;
    must(
        api.call("GET", "/api/me", Some(&old_token), None, None, None)
            .await,
        StatusCode::OK,
        "old session works",
    );
    must(
        api.call(
            "POST",
            "/api/admin/recovery",
            Some(&super_token),
            Some(json!({"username": "emp-revoke", "action": "reset_password", "password": "password-2"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "reset password",
    );
    must(
        api.call("GET", "/api/me", Some(&old_token), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "old cookie revoked after reset",
    );
    let new_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "emp-revoke", "password": "password-2"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "login with new password",
    )
    .token();
    must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "emp-revoke", "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "old password rejected",
    );

    // 顺手：logout 吊销当前会话并清 cookie。
    must(
        api.call("GET", "/api/me", Some(&new_token), None, None, None)
            .await,
        StatusCode::OK,
        "new session works",
    );
    let logged = must(
        api.call(
            "POST",
            "/api/session/logout",
            Some(&new_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "logout",
    );
    let set_cookie = logged
        .cookie
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        set_cookie.contains("max-age=0") || set_cookie.contains("expires="),
        "logout 应清 cookie: {set_cookie}"
    );
    must(
        api.call("GET", "/api/me", Some(&new_token), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "session revoked after logout",
    );

    // 2. TOTP 与密码共用失败计数窗：专用 IP 上 20 次错码后第 21 次 429，
    // 且同用户+IP 的密码登录也被连带限流（共用 user:/ip: 键）。
    let totp_ip: SocketAddr = "10.99.0.1:1234".parse().unwrap();
    for _ in 0..20 {
        must(
            api.call(
                "POST",
                "/api/session/totp",
                None,
                Some(json!({"username": "totp-victim", "code": "000000"})),
                None,
                Some(totp_ip),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "totp wrong code",
        );
    }
    let limited = must(
        api.call(
            "POST",
            "/api/session/totp",
            None,
            Some(json!({"username": "totp-victim", "code": "000000"})),
            None,
            Some(totp_ip),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
        "totp rate limited",
    );
    assert_eq!(limited.json()["error"], "login_rate_limited");
    let shared = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "totp-victim", "password": "whatever-1"})),
            None,
            Some(totp_ip),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
        "password shares totp window",
    );
    assert_eq!(shared.json()["error"], "login_rate_limited");

    // 4. LLM base_url 收紧：http 非回环与 https 私网拒绝，回环 http 与 https 域名放行。
    for (base_url, ok) in [
        ("http://api.openai.com/v1", false),
        ("https://192.168.1.10/v1", false),
        ("https://169.254.169.254/", false),
        ("http://127.0.0.1:8080/v1", true),
        ("https://example.invalid/v1", true),
    ] {
        let res = api
            .call(
                "PUT",
                "/api/admin/llm-channels",
                Some(&super_token),
                Some(json!({"channels": [
                    {"kind": "openai_chat", "api_key": "sk-test", "enabled": true, "base_url": base_url, "model": "demo"}
                ]})),
                None,
                None,
            )
            .await;
        if ok {
            must(res, StatusCode::OK, &format!("base_url 允许 {base_url}"));
        } else {
            let res = must(
                res,
                StatusCode::BAD_REQUEST,
                &format!("base_url 拒绝 {base_url}"),
            );
            assert_eq!(
                res.json()["error"],
                "channel_base_url_invalid",
                "{base_url}"
            );
        }
    }
    // base_url 变更写审计：最近一条 llm_channels 应带 base_url。
    let audits = must(
        api.call(
            "GET",
            "/api/admin/audit",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "audit",
    )
    .json();
    let entry = audits["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["action"] == "llm_channels")
        .expect("应有 llm_channels 审计");
    assert!(
        entry["summary"].to_string().contains("base_url"),
        "审计应记 base_url: {entry}"
    );
    assert!(
        !entry.to_string().contains("sk-test"),
        "审计不能记 key 明文: {entry}"
    );
}

#[tokio::test]
async fn invite_multi_use() {
    let _gate = GATE.lock().await;
    let api = prepare().await;
    let super_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("invite-root"),
            None,
        )
        .await,
        StatusCode::OK,
        "super login",
    )
    .token();

    // 默认一次性：不传 max_uses 即 max_uses=1。
    let once = must(
        api.call(
            "POST",
            "/api/admin/invites",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "invite default",
    )
    .json();
    assert_eq!(once["max_uses"], 1);
    let bad_max = api
        .call(
            "POST",
            "/api/admin/invites",
            Some(&super_token),
            Some(json!({"max_uses": 0})),
            None,
            None,
        )
        .await;
    assert_eq!(bad_max.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_max.json()["error"], "max_uses_invalid");

    // 可用 2 次：两次注册成功，第三次失败。
    let shared = must(
        api.call(
            "POST",
            "/api/admin/invites",
            Some(&super_token),
            Some(json!({"max_uses": 2})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "invite max_uses=2",
    )
    .json();
    let code = shared["code"].as_str().unwrap().to_string();
    let id = shared["id"].as_str().unwrap().to_string();
    for name in ["multi-a", "multi-b"] {
        must(
            api.call(
                "POST",
                "/api/register",
                None,
                Some(json!({"username": name, "invite_code": code, "password": "password-1"})),
                None,
                None,
            )
            .await,
            StatusCode::CREATED,
            "register with shared invite",
        );
    }
    let full = api
        .call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "multi-c", "invite_code": code, "password": "password-1"})),
            None,
            None,
        )
        .await;
    assert_eq!(full.status, StatusCode::BAD_REQUEST);
    assert_eq!(full.json()["error"], "invite_invalid");

    // 列表：已用/上限与明文不泄露。
    let listed = must(
        api.call(
            "GET",
            "/api/admin/invites",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "invite list",
    )
    .json();
    assert!(!listed.to_string().contains(&code), "列表不能出现明文码");
    let row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .expect("应有新建邀请码");
    assert_eq!(row["max_uses"], 2);
    assert_eq!(row["used_count"], 2);
    assert!(row.get("code").is_none());

    // 改次数不得小于已用次数；改大后可继续注册。
    let shrink = api
        .call(
            "PATCH",
            &format!("/api/admin/invites/{id}"),
            Some(&super_token),
            Some(json!({"max_uses": 1})),
            None,
            None,
        )
        .await;
    assert_eq!(shrink.status, StatusCode::BAD_REQUEST);
    assert_eq!(shrink.json()["error"], "max_uses_invalid");
    must(
        api.call(
            "PATCH",
            &format!("/api/admin/invites/{id}"),
            Some(&super_token),
            Some(json!({"max_uses": 3})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "raise max_uses",
    );
    must(
        api.call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "multi-c", "invite_code": code, "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "register after raise",
    );

    // 改小到等于 used_count 即立即用尽。
    must(
        api.call(
            "PATCH",
            &format!("/api/admin/invites/{id}"),
            Some(&super_token),
            Some(json!({"max_uses": 3})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "shrink to used_count",
    );
    let exhausted = api
        .call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "multi-d", "invite_code": code, "password": "password-1"})),
            None,
            None,
        )
        .await;
    assert_eq!(exhausted.status, StatusCode::BAD_REQUEST);

    // 作废后不能再注册，列表标已作废。
    must(
        api.call(
            "DELETE",
            &format!("/api/admin/invites/{id}"),
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "revoke",
    );
    let revoked = api
        .call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "multi-e", "invite_code": code, "password": "password-1"})),
            None,
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::BAD_REQUEST);
    assert_eq!(revoked.json()["error"], "invite_invalid");
    let listed = must(
        api.call(
            "GET",
            "/api/admin/invites",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "invite list after revoke",
    )
    .json();
    let row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .expect("应有新建邀请码");
    assert!(
        row["revoked_at"].is_string(),
        "作废后列表应标 revoked_at: {row}"
    );
}

#[tokio::test]
async fn invite_role() {
    let _gate = GATE.lock().await;
    let api = prepare().await;
    let super_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("invite-role-root"),
            None,
        )
        .await,
        StatusCode::OK,
        "super login",
    )
    .token();

    let canteen = uid(
        &must(
            api.call(
                "POST",
                "/api/admin/canteens",
                Some(&super_token),
                Some(json!({"name": "邀请食堂"})),
                None,
                None,
            )
            .await,
            StatusCode::CREATED,
            "canteen",
        )
        .json(),
        "id",
    );

    async fn login(api: &Api, username: &str) -> String {
        must(
            api.call(
                "POST",
                "/api/session/password",
                None,
                Some(json!({"username": username, "password": "password-1"})),
                Some(username),
                None,
            )
            .await,
            StatusCode::OK,
            "login",
        )
        .token()
    }

    // 默认仍是员工码：不传 role 即 employee，注册后只有 employee 身份。
    let emp_code = must(
        api.call(
            "POST",
            "/api/admin/invites",
            Some(&super_token),
            Some(json!({"max_uses": 2})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "employee invite",
    )
    .json();
    assert_eq!(emp_code["role"], "employee");
    assert_eq!(emp_code["canteen_ids"], json!([]));
    must(
        api.call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "role-emp", "invite_code": emp_code["code"], "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "register employee",
    );
    let emp_token = login(&api, "role-emp").await;
    let me = must(
        api.call("GET", "/api/me", Some(&emp_token), None, None, None)
            .await,
        StatusCode::OK,
        "me employee",
    )
    .json();
    assert_eq!(me["roles"], json!([{"role": "employee", "canteen_ids": [], "source_label": Value::Null}]));

    // 食堂码：注册成 canteen + 范围，并同步写入 canteen_staff。
    let can_code = must(
        api.call(
            "POST",
            "/api/admin/invites",
            Some(&super_token),
            Some(json!({"role": "canteen", "canteen_ids": [canteen]})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "canteen invite",
    )
    .json();
    assert_eq!(can_code["role"], "canteen");
    must(
        api.call(
            "POST",
            "/api/register",
            None,
            Some(json!({"username": "role-can", "invite_code": can_code["code"], "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::CREATED,
        "register canteen",
    );
    let can_token = login(&api, "role-can").await;
    let me = must(
        api.call("GET", "/api/me", Some(&can_token), None, None, None)
            .await,
        StatusCode::OK,
        "me canteen",
    )
    .json();
    assert_eq!(
        me["roles"],
        json!([{"role": "canteen", "canteen_ids": [canteen.to_string()], "source_label": Value::Null}])
    );
    let staff: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM canteen_staff WHERE account_id = (SELECT id FROM account WHERE username = 'role-can') AND canteen_id = $1)",
    )
    .bind(canteen)
    .fetch_one(&api.pool)
    .await
    .unwrap();
    assert!(staff, "食堂邀请注册应同步写入 canteen_staff");

    // 列表展示角色与范围。
    let listed = must(
        api.call(
            "GET",
            "/api/admin/invites",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "invite list",
    )
    .json();
    let can_row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == can_code["id"])
        .expect("应有食堂邀请码");
    assert_eq!(can_row["role"], "canteen");
    assert_eq!(can_row["canteen_ids"], json!([canteen.to_string()]));
    assert!(can_row.get("code").is_none());

    // 非法：admin/super_admin 不能经邀请码发；employee 带范围被拒；canteen 空范围被拒；不存在的食堂被拒。
    for body in [
        json!({"role": "admin"}),
        json!({"role": "super_admin"}),
        json!({"role": "boss"}),
        json!({"role": "employee", "canteen_ids": [canteen]}),
        json!({"role": "canteen"}),
        json!({"role": "canteen", "canteen_ids": [Uuid::new_v4()]}),
    ] {
        let bad = api
            .call(
                "POST",
                "/api/admin/invites",
                Some(&super_token),
                Some(body),
                None,
                None,
            )
            .await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST, "非法邀请应被拒: {}", bad.json());
    }
}

#[tokio::test]
async fn account_settings() {
    let _gate = GATE.lock().await;
    let api = prepare().await;
    let super_token = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "root", "password": "root-password-1"})),
            Some("acct-root"),
            None,
        )
        .await,
        StatusCode::OK,
        "super login",
    )
    .token();
    let (token_a, _) = register_user(&api, &super_token, "acct-a").await;
    let token_b = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "acct-a", "password": "password-1"})),
            Some("acct-a-other"),
            None,
        )
        .await,
        StatusCode::OK,
        "second session",
    )
    .token();
    assert_ne!(token_a, token_b);

    let me = must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "me before rename",
    )
    .json();
    assert_eq!(me["username"], "acct-a");
    assert!(me.get("display_name").unwrap().is_null());
    assert_eq!(me["email_verified"], false);
    assert_eq!(me["totp"], false);

    must(
        api.call(
            "PUT",
            "/api/me/display-name",
            None,
            Some(json!({"display_name": "外人"})),
            None,
            None,
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "rename requires session",
    );
    let renamed = must(
        api.call(
            "PUT",
            "/api/me/display-name",
            Some(&token_a),
            Some(json!({"display_name": "  灶上的人  "})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "rename",
    )
    .json();
    assert_eq!(renamed["display_name"], "灶上的人");
    let me = must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "me after rename",
    )
    .json();
    assert_eq!(me["display_name"], "灶上的人");
    assert_eq!(me["username"], "acct-a");
    let cleared = must(
        api.call(
            "PUT",
            "/api/me/display-name",
            Some(&token_a),
            Some(json!({"display_name": "   "})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "clear display name",
    )
    .json();
    assert!(cleared["display_name"].is_null());
    must(
        api.call(
            "PUT",
            "/api/me/display-name",
            Some(&token_a),
            Some(json!({"display_name": "名".repeat(65)})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "display name too long",
    );
    must(
        api.call(
            "PUT",
            "/api/me/display-name",
            Some(&token_a),
            Some(json!({"display_name": "灶上的人"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "rename again",
    );
    let started = must(
        api.call(
            "POST",
            "/api/passkeys/register/start",
            Some(&token_a),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "passkey start uses display name",
    )
    .json();
    let started_text = started.to_string();
    assert!(
        started_text.contains("灶上的人"),
        "Passkey 展示名应使用显示名: {started_text}"
    );
    assert!(
        started_text.contains("acct-a"),
        "Passkey 登录名仍是账号: {started_text}"
    );

    let accounts = must(
        api.call(
            "GET",
            "/api/admin/accounts",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "accounts",
    )
    .json();
    assert!(!accounts.to_string().contains("password_hash"));
    let acct = accounts["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["username"] == "acct-a")
        .unwrap();
    assert_eq!(acct["display_name"], "灶上的人");

    let wrong = must(
        api.call(
            "POST",
            "/api/me/password",
            Some(&token_a),
            Some(json!({"current_password": "nope-xxxx", "new_password": "password-2"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "wrong current password",
    );
    assert_eq!(wrong.json()["error"], "password_incorrect");
    must(
        api.call("GET", "/api/me", Some(&token_b), None, None, None)
            .await,
        StatusCode::OK,
        "wrong password does not revoke",
    );
    let weak = must(
        api.call(
            "POST",
            "/api/me/password",
            Some(&token_a),
            Some(json!({"current_password": "password-1", "new_password": "short"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "weak new password",
    );
    assert_eq!(weak.json()["error"], "password_weak");
    let same = must(
        api.call(
            "POST",
            "/api/me/password",
            Some(&token_a),
            Some(json!({"current_password": "password-1", "new_password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "same password",
    );
    assert_eq!(same.json()["error"], "password_unchanged");
    let changed = must(
        api.call(
            "POST",
            "/api/me/password",
            Some(&token_a),
            Some(json!({"current_password": "password-1", "new_password": "password-2"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "change password",
    );
    assert!(changed.cookie.is_none(), "自助改密不轮换当前 cookie");
    assert!(changed.json()["revoked_other_sessions"].as_u64().unwrap() >= 1);
    must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "current session kept",
    );
    must(
        api.call("GET", "/api/me", Some(&token_b), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "other session revoked",
    );
    let devices = must(
        api.call("GET", "/api/devices", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "devices remain",
    )
    .json();
    let device_rows = devices["devices"].as_array().unwrap();
    assert!(
        device_rows.len() >= 2,
        "改密按会话吊销，不删设备: {devices}"
    );
    assert_eq!(
        device_rows
            .iter()
            .filter(|item| item["session_active"] == true)
            .count(),
        1
    );
    must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "acct-a", "password": "password-1"})),
            None,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "old password rejected",
    );
    let token_c = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "acct-a", "password": "password-2"})),
            Some("acct-a-new"),
            None,
        )
        .await,
        StatusCode::OK,
        "login with new password",
    )
    .token();
    must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "original session still valid after a new login",
    );

    must(
        api.call(
            "PUT",
            "/api/admin/settings/email",
            Some(&super_token),
            Some(json!({"enabled": true})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "enable email",
    );
    must(
        api.call(
            "PUT",
            "/api/me/email",
            Some(&token_a),
            Some(json!({"email": "acct-a@example.com"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "bind email",
    );
    let me = must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "email bound",
    )
    .json();
    assert_eq!(me["email"], "acct-a@example.com");
    assert_eq!(me["email_verified"], false);
    must(
        api.call(
            "POST",
            "/api/session/email-otp/request",
            None,
            Some(json!({"username": "acct-a"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "otp before password change",
    );
    let mail = api.mailer.messages.lock().unwrap().clone();
    let pending_code = otp_code(&mail.last().unwrap().2);
    must(
        api.call(
            "POST",
            "/api/me/password",
            Some(&token_a),
            Some(json!({"current_password": "password-2", "new_password": "password-3"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "change password again",
    );
    let voided = api
        .call(
            "POST",
            "/api/session/email-otp/verify",
            None,
            Some(json!({"username": "acct-a", "code": pending_code})),
            None,
            None,
        )
        .await;
    assert_eq!(voided.status, StatusCode::BAD_REQUEST);
    assert_eq!(voided.json()["error"], "otp_invalid");
    must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "keeper session after second change",
    );
    must(
        api.call("GET", "/api/me", Some(&token_c), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "session from previous password revoked",
    );

    let setup = must(
        api.call(
            "POST",
            "/api/me/totp/setup",
            Some(&token_a),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "totp setup",
    )
    .json();
    let secret = totp_rs::Secret::Encoded(setup["secret"].as_str().unwrap().to_string())
        .to_bytes()
        .unwrap();
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some("多邻灶".into()),
        "acct-a".into(),
    )
    .unwrap();
    let now = Utc
        .with_ymd_and_hms(2026, 10, 10, 2, 0, 0)
        .unwrap()
        .timestamp() as u64;
    must(
        api.call(
            "POST",
            "/api/me/totp/confirm",
            Some(&token_a),
            Some(json!({"code": totp.generate(now)})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "totp confirm",
    );
    let me = must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::OK,
        "totp flag",
    )
    .json();
    assert_eq!(me["totp"], true);

    let logged = must(
        api.call(
            "POST",
            "/api/session/logout",
            Some(&token_a),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "logout",
    );
    let set_cookie = logged
        .cookie
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        set_cookie.contains("max-age=0") || set_cookie.contains("expires="),
        "logout clears cookie: {set_cookie}"
    );
    must(
        api.call("GET", "/api/me", Some(&token_a), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "logged out",
    );

    let token_d = must(
        api.call(
            "POST",
            "/api/session/password",
            None,
            Some(json!({"username": "acct-a", "password": "password-3"})),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "login after self change",
    )
    .token();
    must(
        api.call(
            "POST",
            "/api/admin/recovery",
            Some(&super_token),
            Some(
                json!({"username": "acct-a", "action": "reset_password", "password": "password-4"}),
            ),
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "super recovery still revokes every session",
    );
    must(
        api.call("GET", "/api/me", Some(&token_d), None, None, None)
            .await,
        StatusCode::UNAUTHORIZED,
        "recovery revokes the current session too",
    );

    let audit = must(
        api.call(
            "GET",
            "/api/admin/audit",
            Some(&super_token),
            None,
            None,
            None,
        )
        .await,
        StatusCode::OK,
        "audit",
    )
    .json();
    let audit_text = audit.to_string();
    assert!(audit_text.contains("display_name_update"));
    assert!(audit_text.contains("password_change"));
    assert!(!audit_text.contains("password-2"));
    assert!(!audit_text.contains("password-3"));
    assert!(!audit_text.contains("password_hash"));
}
