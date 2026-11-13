use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::error::AppError;
use crate::extract::{ClientIp, CurrentUser, UserAgent};
use crate::ops::{admin, app_signing, catalog, meal, session};
use crate::state::AppState;

/// 写操作第二道闸：状态变更方法必须带自定义头，证明是同源 JS 发起。
/// GET/HEAD/OPTIONS 直接放行（覆盖纯静态资源与健康检查）；
/// 其余方法一律要求 `X-DLZ-Requested: 1`，包括登录/注册等无会话端点——前端已统一带上。
async fn csrf_guard(request: Request, next: Next) -> Result<Response, AppError> {
    let unsafe_method = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if unsafe_method {
        let ok = request
            .headers()
            .get("x-dlz-requested")
            .and_then(|value| value.to_str().ok())
            == Some("1");
        if !ok {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "csrf_required",
                "缺少 CSRF 校验头",
            ));
        }
    }
    Ok(next.run(request).await)
}

pub fn router(state: AppState) -> Router {
    let limit = state.config.dish_image_max_bytes as usize + 65_536;
    let mut api = Router::new()
        .route("/api/health", get(health))
        // 部署探针：Traefik/CI 只看连通性，不查库、不鉴权，免 CSRF（GET 豁免）。
        .route("/healthz", get(healthz))
        // 就绪探针已迁往回环管理端口（见 admin_router），公网不再挂载，避免外部直达 DB 探针。
        // Digital Asset Links：Android/Google 匿名拉取，必须免会话且是 application/json，
        // 不能被文末的 SPA fallback 吃成 index.html（否则通行密钥判「站点与 App 无关联」）。
        .route("/.well-known/assetlinks.json", get(assetlinks))
        .route("/api/session/methods", get(methods))
        .route("/api/register", post(register))
        .route("/api/session/password", post(login_password))
        .route("/api/session/totp", post(login_totp))
        .route("/api/session/logout", post(logout))
        .route("/api/session/email-otp/request", post(email_otp_request))
        .route("/api/session/email-otp/verify", post(email_otp_verify))
        // 站点自托管 `/app/`：通用 APK 与 version.json。显式路由优先于文末的
        // SPA fallback，缺文件时返回 404 JSON（不回退 index.html，见设计 §4.4）。
        .route("/app/*path", get(serve_app_file))
        .route("/api/session/passkey/start", post(passkey_start))
        .route("/api/session/passkey/finish", post(passkey_finish))
        .route("/api/passkeys/register/start", post(passkey_register_start))
        .route(
            "/api/passkeys/register/finish",
            post(passkey_register_finish),
        )
        .route("/api/passkeys/:id", axum::routing::delete(passkey_delete))
        .route("/api/login-approvals", post(approval_create))
        .route("/api/login-approvals/pending", get(approval_pending))
        .route("/api/login-approvals/:id", get(approval_poll))
        .route("/api/login-approvals/:id/allow", post(approval_allow))
        .route("/api/login-approvals/:id/deny", post(approval_deny))
        .route("/api/devices", get(devices))
        .route("/api/devices/:id", axum::routing::delete(device_delete))
        .route("/api/me", get(me))
        .route("/api/me/display-name", put(put_display_name))
        .route("/api/me/password", post(change_password))
        .route("/api/me/email", put(bind_email))
        .route("/api/me/totp/setup", post(totp_setup))
        .route("/api/me/totp/confirm", post(totp_confirm))
        .route("/api/me/canteens", put(bind_canteens).get(my_canteens))
        .route("/api/me/shift", put(put_shift).get(get_shift))
        .route("/api/me/meals", get(my_meals))
        .route("/api/me/home", get(my_home))
        .route("/api/me/meal-template", get(my_template))
        .route("/api/me/score-targets", get(score_targets))
        .route("/api/meals/:date/:slot", put(put_meal))
        .route("/api/meal-templates/generate", post(generate_template))
        .route("/api/meal-templates/:id", put(put_template))
        .route("/api/meal-templates/:id/slots", put(put_slot))
        .route("/api/meal-templates/:id/apply", post(apply_template))
        .route("/api/canteens", get(list_canteens))
        .route(
            "/api/canteens/:id/dishes",
            get(list_dishes).post(create_dish),
        )
        .route("/api/canteens/:id/dishes/:dish_id", put(update_dish))
        .route(
            "/api/canteens/:id/dishes/:dish_id/image",
            post(upload_image).delete(delete_image),
        )
        .route("/api/dishes/:id/image", get(read_image))
        .route("/api/canteens/:id/headcount", get(headcount))
        .route("/api/canteens/:id/alerts", get(alerts))
        .route("/api/canteens/:id/selection/week", get(selection_week))
        .route("/api/canteens/:id/selection", get(selection_slot))
        .route(
            "/api/canteens/:id/selection/:date/:slot",
            put(put_selection),
        )
        .route("/api/scores", post(create_score))
        .route("/api/notifications", get(notifications))
        .route("/api/notifications/:id/read", post(read_notification))
        .route("/api/push/reservations", post(push_reserved))
        .route("/api/admin/canteens", post(create_canteen))
        .route("/api/admin/role-assignments", post(assign_role))
        .route("/api/admin/invites", post(create_invite).get(list_invites))
        .route(
            "/api/admin/invites/:id",
            patch(update_invite).delete(revoke_invite),
        )
        .route("/api/admin/meals", get(admin_meals))
        .route(
            "/api/admin/meals/:account_id/:date/:slot",
            put(admin_put_meal),
        )
        .route("/api/admin/comments", get(admin_comments))
        .route("/api/admin/score-summaries", get(admin_scores))
        .route("/api/admin/settings/email", get(get_email).put(put_email))
        .route(
            "/api/admin/settings/score-alert",
            get(get_alert).put(put_alert),
        )
        .route(
            "/api/admin/llm-channels",
            get(get_channels).put(put_channels),
        )
        // 应用签名：keystore 只活在数据库（空库可自动生成），由超管上传覆盖；正式签名在服务端完成。
        // 注意：绝不加导出/下载路由（keystore 本体与密码永不经 API 回传）。
        .route("/api/admin/app-signing", get(get_app_signing).put(put_app_signing))
        .route("/api/admin/app-signing/generate", post(generate_app_signing))
        .route("/api/admin/app-signing/sign", post(sign_app_release))
        .route("/api/admin/recovery", post(recover))
        .route("/api/admin/accounts", get(accounts))
        .route("/api/admin/audit", get(audit))
        .route("/api/admin/exports", post(export_zip))
        // 测试时钟（仅测试环境入口）：拨 T0–T5 切片，不走业务 API 造数据。
        .route("/api/test/clock", get(get_test_clock).post(set_test_clock))
        .layer(DefaultBodyLimit::max(limit))
        .layer(TraceLayer::new_for_http())
        // CSRF 闸 + 安全响应头（后加的 layer 包在外层，拒绝响应也带安全头）。
        // CSP 保守：脚本限同源，样式允许内联（Element Plus 动态样式需要），不破坏现有 Vue 应用。
        .layer(middleware::from_fn(csrf_guard))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; base-uri 'self'; frame-ancestors 'self'; \
                 img-src 'self' data: blob:; font-src 'self' data:; \
                 style-src 'self' 'unsafe-inline'; script-src 'self'; \
                 connect-src 'self'; object-src 'none'",
            ),
        ))
        .with_state(state.clone());
    if let Some(dir) = state.config.static_dir.clone() {
        let index = format!("{dir}/index.html");
        let serve = ServeDir::new(&dir).not_found_service(ServeFile::new(index));
        api = api.fallback_service(serve);
    }
    api
}

/// 管理回环路由：只挂 `/readyz`，由 `main` 绑在 `server.admin_bind`（缺省 127.0.0.1:8090）。
/// 边界靠只绑回环实现：外部到不了，故不判 IP、不加 token——
/// XFF 首段可伪造（判了等于没判），token 则多一套密钥分发与轮换（已和用户对齐不要）。
/// 公网 router 绝不挂载此路由。
pub fn admin_router(state: AppState) -> Router {
    Router::new()
        .route("/readyz", get(readyz))
        .with_state(state)
}

fn session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new("dlz_session", token.to_string());
    cookie.set_http_only(true);
    cookie.set_path("/");
    cookie.set_same_site(SameSite::Lax);
    // 持久 Cookie：浏览器 / WebView 退出进程后仍保留，配合服务端 180 天会话。
    // 满足「很久不掉线」；吊销仍走设备/会话删除。
    cookie.set_max_age(cookie::time::Duration::days(crate::ops::SESSION_DAYS));
    if secure {
        cookie.set_secure(true);
    }
    cookie
}

fn with_token(token: &str, secure: bool, body: Value) -> impl IntoResponse {
    let jar = CookieJar::new().add(session_cookie(token, secure));
    (jar, Json(body))
}

fn cleared_cookie(secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new("dlz_session", "");
    cookie.set_http_only(true);
    cookie.set_path("/");
    cookie.set_same_site(SameSite::Lax);
    if secure {
        cookie.set_secure(true);
    }
    cookie.make_removal();
    cookie
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok", "name": "多邻灶", "version": crate::version::app_version()}))
}

async fn healthz() -> &'static str {
    "ok"
}

/// 就绪探针 DB 短超时：readiness 必须快失败。不加 timeout 时 DB 断掉会被 pool 的
/// acquire_timeout 拖住（默认 30s，CI 单测实测约 30s 才返回 503），Swarm 健康检查
/// 与排障都拿不到及时信号；2s 足够本地/同机网络，超时按 db_unavailable 处理。
const READYZ_DB_TIMEOUT: Duration = Duration::from_secs(2);

/// 就绪探针：`GET /readyz`（仅挂管理回环 `admin_router`，公网无此路由），
/// `SELECT 1` 走 pool：正常 200 `{status:ok}`，DB 错/超时 503 `{error:db_unavailable}`（仅此端点用新码，其它接口错误契约不动）。
/// 不鉴权、不判 IP、不加 token：回环绑定即边界。
async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let probe = sqlx::query("SELECT 1").fetch_one(&state.pool);
    match tokio::time::timeout(READYZ_DB_TIMEOUT, probe).await {
        Ok(Ok(_)) => (StatusCode::OK, Json(json!({"status": "ok"}))).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "readyz database");
            db_unavailable()
        }
        Err(_) => {
            tracing::error!(
                timeout_ms = READYZ_DB_TIMEOUT.as_millis() as u64,
                "readyz database timeout"
            );
            db_unavailable()
        }
    }
}

/// DB 不可用的统一响应：错误契约固定为 `db_unavailable`（时间超时与连接失败同码）。
fn db_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "db_unavailable", "message": "数据库不可用"})),
    )
        .into_response()
}

/// `/.well-known/assetlinks.json`：指纹取「签名发布」写进 `version.json` 的 release
/// 证书（`certSha256`），所以永远与实际发放的包一致；未签名时回 `[]`（合法 JSON =
/// 没有关联的 App）。公开端点：系统校验器不带 cookie。
async fn assetlinks(State(state): State<AppState>) -> impl IntoResponse {
    let cert = state
        .config
        .resolved_app_dir()
        .and_then(|dir| std::fs::read_to_string(format!("{dir}/version.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|meta| {
            meta.get("certSha256")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(app_signing::assetlinks(cert.as_deref())),
    )
}

/// 站点自托管 `/app/`（通用 APK 与 version.json，供应用内更新同源拉取）。
/// **要求有效会话**：匿名一律 401。首次装包入口只在超管「应用签名」页（复制链接/下载）；
/// 员工「我的」、食堂「更多」、登录页、页脚不放入口。已登录用户可检查更新并拉 APK。
/// 文件来自 `app_dir`（`[server] app_dir` 或 `DLZ_APP_DIR`，缺省 `{static_dir}/app/`）；
/// 缺文件、未配目录或路径逃逸一律 404 JSON，绝不回退 index.html
/// （否则 App 会把 HTML 当 version.json 解析失败）。
async fn serve_app_file(
    State(state): State<AppState>,
    _user: CurrentUser,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "不存在"})),
        )
            .into_response()
    };
    let Some(dir) = state.config.resolved_app_dir() else {
        return not_found();
    };
    if path.contains("..") || path.contains('\\') || path.starts_with('/') {
        return not_found();
    }
    let full = format!("{dir}/{path}");
    let bytes = match tokio::fs::read(&full).await {
        Ok(item) => item,
        Err(_) => return not_found(),
    };
    let content_type = if full.ends_with(".apk") {
        "application/vnd.android.package-archive"
    } else if full.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    };
    // version.json 不缓存；APK 可私有缓存一小时（带会话才拿到，别让共享缓存回放给匿名）。
    let cache = if full.ends_with("version.json") {
        "no-store"
    } else {
        "private, max-age=3600"
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache),
        ],
        bytes,
    )
        .into_response()
}

async fn methods(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(session::methods(&state).await?))
}

#[derive(Deserialize)]
struct RegisterBody {
    username: String,
    invite_code: String,
    /// 省略、null 或空白表示不设密码。
    #[serde(default)]
    password: Option<String>,
}

async fn register(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, AppError> {
    // 注册成功即登录：直接带上会话 cookie，不用再登一次。
    let (created, issued) = session::register(
        &state,
        &body.username,
        &body.invite_code,
        body.password.as_deref(),
        &ua.0,
        &ip.0,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        with_token(&issued.raw_token, state.config.cookie_secure, created),
    ))
}

#[derive(Deserialize)]
struct PasswordBody {
    username: String,
    password: String,
}

async fn login_password(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<PasswordBody>,
) -> Result<impl IntoResponse, AppError> {
    let issued =
        session::login_password(&state, &body.username, &body.password, &ua.0, &ip.0).await?;
    Ok(with_token(
        &issued.raw_token,
        state.config.cookie_secure,
        json!({"ok": true}),
    ))
}

#[derive(Deserialize)]
struct TotpBody {
    username: String,
    code: String,
}

async fn login_totp(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<TotpBody>,
) -> Result<impl IntoResponse, AppError> {
    let issued = session::login_totp(&state, &body.username, &body.code, &ua.0, &ip.0).await?;
    Ok(with_token(
        &issued.raw_token,
        state.config.cookie_secure,
        json!({"ok": true}),
    ))
}

async fn logout(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<impl IntoResponse, AppError> {
    let body = session::logout(&state, &user).await?;
    let jar = CookieJar::new().add(cleared_cookie(state.config.cookie_secure));
    Ok((jar, Json(body)))
}

#[derive(Deserialize)]
struct EmailOtpRequest {
    username: String,
}

async fn email_otp_request(
    State(state): State<AppState>,
    Json(body): Json<EmailOtpRequest>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::request_email_otp(&state, &body.username).await?,
    ))
}

#[derive(Deserialize)]
struct EmailOtpVerify {
    username: String,
    code: String,
}

async fn email_otp_verify(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<EmailOtpVerify>,
) -> Result<impl IntoResponse, AppError> {
    let issued =
        session::verify_email_otp(&state, &body.username, &body.code, &ua.0, &ip.0).await?;
    Ok(with_token(
        &issued.raw_token,
        state.config.cookie_secure,
        json!({"ok": true}),
    ))
}

#[derive(Deserialize)]
struct PasskeyStart {
    #[serde(default)]
    username: Option<String>,
}

async fn passkey_start(
    State(state): State<AppState>,
    Json(body): Json<PasskeyStart>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::passkey_auth_start(&state, body.username.as_deref()).await?,
    ))
}

#[derive(Deserialize)]
struct PasskeyFinish {
    #[serde(default)]
    username: Option<String>,
    challenge_id: Uuid,
    credential: webauthn_rs::prelude::PublicKeyCredential,
}

async fn passkey_finish(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<PasskeyFinish>,
) -> Result<impl IntoResponse, AppError> {
    let issued = session::passkey_auth_finish(
        &state,
        body.username.as_deref(),
        body.challenge_id,
        body.credential,
        &ua.0,
        &ip.0,
    )
    .await?;
    Ok(with_token(
        &issued.raw_token,
        state.config.cookie_secure,
        json!({"ok": true}),
    ))
}

async fn passkey_register_start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::passkey_register_start(&state, &user).await?))
}

#[derive(Deserialize)]
struct PasskeyRegFinish {
    challenge_id: Uuid,
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
    friendly_name: Option<String>,
}

async fn passkey_register_finish(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<PasskeyRegFinish>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::passkey_register_finish(
            &state,
            &user,
            body.challenge_id,
            body.credential,
            body.friendly_name,
        )
        .await?,
    ))
}

async fn passkey_delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::unbind_passkey(&state, &user, id).await?))
}

#[derive(Deserialize)]
struct ApprovalCreate {
    username: String,
}

async fn approval_create(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<ApprovalCreate>,
) -> Result<impl IntoResponse, AppError> {
    let created = session::create_approval(&state, &body.username, &ua.0, &ip.0).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn approval_pending(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::pending_approvals(&state, &user).await?))
}

#[derive(Deserialize)]
struct PollQuery {
    poll_token: String,
}

async fn approval_poll(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<PollQuery>,
) -> Result<impl IntoResponse, AppError> {
    let (body, token) = session::poll_approval(&state, id, &query.poll_token).await?;
    if let Some(token) = token {
        Ok(with_token(&token, state.config.cookie_secure, body).into_response())
    } else {
        Ok(Json(body).into_response())
    }
}

async fn approval_allow(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::allow_approval(&state, &user, id).await?))
}

async fn approval_deny(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::deny_approval(&state, &user, id).await?))
}

async fn devices(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::list_devices(&state, &user).await?))
}

async fn device_delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::kick_device(&state, &user, id).await?))
}

async fn me(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    Ok(Json(session::me(&state, &user).await?))
}

#[derive(Deserialize)]
struct DisplayNameBody {
    display_name: String,
}

async fn put_display_name(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<DisplayNameBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::update_display_name(&state, &user, &body.display_name).await?,
    ))
}

#[derive(Deserialize)]
struct PasswordChangeBody {
    /// 尚无密码时可以省略。已有密码时必须是当前密码。
    #[serde(default)]
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(state): State<AppState>,
    user: CurrentUser,
    ip: ClientIp,
    Json(body): Json<PasswordChangeBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::change_password(
            &state,
            &user,
            &body.current_password,
            &body.new_password,
            &ip.0,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct EmailBody {
    email: String,
}

async fn bind_email(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::bind_email(&state, &user, &body.email).await?))
}

async fn totp_setup(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(session::totp_setup(&state, &user).await?))
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

async fn totp_confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<CodeBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::totp_confirm(&state, &user, &body.code).await?,
    ))
}

#[derive(Deserialize)]
struct CanteensBody {
    canteen_ids: Vec<Uuid>,
}

async fn bind_canteens(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<CanteensBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::bind_canteens(&state, &user, &body.canteen_ids).await?,
    ))
}

async fn my_canteens(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(meal::my_canteens(&state, &user).await?))
}

#[derive(Deserialize)]
struct ShiftBody {
    kind: String,
    anchor_date: Option<NaiveDate>,
    canteen_id: Option<Uuid>,
}

async fn put_shift(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ShiftBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::put_shift(&state, &user, &body.kind, body.anchor_date, body.canteen_id).await?,
    ))
}

async fn get_shift(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(meal::get_shift(&state, &user).await?))
}

#[derive(Deserialize)]
struct RangeQuery {
    start: NaiveDate,
    end: NaiveDate,
}

async fn my_meals(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<RangeQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::list_meals(&state, &user, query.start, query.end).await?,
    ))
}

async fn my_home(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(meal::home_board(&state, &user).await?))
}

async fn my_template(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(meal::get_template(&state, &user).await?))
}

#[derive(Deserialize)]
struct MealBody {
    ordering: bool,
    canteen_id: Option<Uuid>,
}

async fn put_meal(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((date, slot)): Path<(NaiveDate, String)>,
    Json(body): Json<MealBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::put_meal(&state, &user, date, &slot, body.ordering, body.canteen_id).await?,
    ))
}

#[derive(Deserialize)]
struct GenerateBody {
    description: String,
    anchor_date: Option<NaiveDate>,
    canteen_id: Option<Uuid>,
}

async fn generate_template(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<GenerateBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::generate_template(
            &state,
            &user,
            &body.description,
            body.anchor_date,
            body.canteen_id,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct TemplateBody {
    dsl: crate::domain::MealDsl,
}

async fn put_template(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TemplateBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(meal::put_template(&state, &user, id, body.dsl).await?))
}

#[derive(Deserialize)]
struct SlotBody {
    date: NaiveDate,
    slot: String,
    ordering: bool,
    canteen_id: Option<Uuid>,
}

async fn put_slot(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SlotBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::put_slot(
            &state,
            &user,
            id,
            body.date,
            &body.slot,
            body.ordering,
            body.canteen_id,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct ApplyBody {
    start_date: NaiveDate,
    end_date: NaiveDate,
}

async fn apply_template(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::apply_template(&state, &user, id, body.start_date, body.end_date).await?,
    ))
}

async fn list_canteens(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::list_canteens(&state, &user).await?))
}

#[derive(Deserialize)]
struct KindQuery {
    kind: Option<String>,
}

async fn list_dishes(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<KindQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::list_dishes(&state, &user, id, query.kind.as_deref()).await?,
    ))
}

#[derive(Deserialize)]
struct DishBody {
    name: String,
    kind: String,
    #[serde(default)]
    taste_tags: Vec<String>,
    enabled: Option<bool>,
    canteen_id: Option<Uuid>,
}

fn dish_input(body: DishBody) -> catalog::DishInput {
    catalog::DishInput {
        name: body.name,
        kind: body.kind,
        taste_tags: body.taste_tags,
        enabled: body.enabled,
        canteen_id: body.canteen_id,
    }
}

async fn create_dish(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<DishBody>,
) -> Result<impl IntoResponse, AppError> {
    let created = catalog::create_dish(&state, &user, id, dish_input(body)).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update_dish(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, dish_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<DishBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::update_dish(&state, &user, id, dish_id, dish_input(body)).await?,
    ))
}

async fn upload_image(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, dish_id)): Path<(Uuid, Uuid)>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let mut bytes = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::bad("image_invalid", "无法读取上传"))?
    {
        if field.name() == Some("file") || field.name() == Some("image") {
            bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|_| AppError::bad("image_invalid", "无法读取上传"))?,
            );
        }
    }
    let bytes = bytes.ok_or_else(|| AppError::bad("image_required", "需要上传图片"))?;
    Ok(Json(
        catalog::save_image(&state, &user, id, dish_id, &bytes).await?,
    ))
}

async fn delete_image(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, dish_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::remove_image(&state, &user, id, dish_id).await?,
    ))
}

async fn read_image(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let (bytes, content_type) = catalog::read_image(&state, &user, id).await?;
    Ok(([(header::CONTENT_TYPE, content_type)], bytes))
}

#[derive(Deserialize)]
struct DateQuery {
    date: NaiveDate,
}

async fn headcount(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<DateQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::headcount(&state, &user, id, query.date).await?,
    ))
}

async fn alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(catalog::alerts(&state, &user, id).await?))
}

async fn selection_week(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<DateQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::selection_week(&state, &user, id, query.date).await?,
    ))
}

#[derive(Deserialize)]
struct SlotQuery {
    date: NaiveDate,
    slot: String,
}

async fn selection_slot(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<SlotQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::selection_slot(&state, &user, id, query.date, &query.slot).await?,
    ))
}

#[derive(Deserialize)]
struct SelectionBody {
    dish_ids: Vec<Uuid>,
}

async fn put_selection(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, date, slot)): Path<(Uuid, NaiveDate, String)>,
    Json(body): Json<SelectionBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::put_selection(&state, &user, id, date, &slot, &body.dish_ids).await?,
    ))
}

#[derive(Deserialize)]
struct ScoreBody {
    meal_date: NaiveDate,
    slot: String,
    dish_id: Uuid,
    value: Option<i16>,
    comment: Option<String>,
    #[serde(default)]
    anonymous: bool,
}

async fn create_score(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ScoreBody>,
) -> Result<impl IntoResponse, AppError> {
    let created = catalog::submit_score(
        &state,
        &user,
        body.meal_date,
        &body.slot,
        body.dish_id,
        body.value,
        body.comment,
        body.anonymous,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(Deserialize)]
struct TargetQuery {
    date: NaiveDate,
    slot: String,
}

async fn score_targets(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<TargetQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::score_targets(&state, &user, query.date, &query.slot).await?,
    ))
}

#[derive(Deserialize)]
struct CanteenKindQuery {
    canteen_id: Uuid,
    kind: Option<String>,
}

async fn admin_comments(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<CanteenKindQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::comments(&state, &user, query.canteen_id, query.kind.as_deref()).await?,
    ))
}

async fn admin_scores(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<CanteenKindQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        catalog::score_summaries(&state, &user, query.canteen_id, query.kind.as_deref()).await?,
    ))
}

async fn notifications(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(catalog::notifications(&state, &user).await?))
}

async fn read_notification(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(catalog::read_notification(&state, &user, id).await?))
}

async fn push_reserved() -> AppError {
    AppError::new(
        StatusCode::NOT_IMPLEMENTED,
        "push_not_in_phase1",
        "一期不投递推送",
    )
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn create_canteen(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<NameBody>,
) -> Result<impl IntoResponse, AppError> {
    let created = admin::create_canteen(&state, &user, &body.name).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(Deserialize)]
struct RoleBody {
    account_id: Uuid,
    role: String,
    #[serde(default)]
    canteen_ids: Vec<Uuid>,
    source_label: Option<String>,
}

async fn assign_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<RoleBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        admin::assign_role(
            &state,
            &user,
            body.account_id,
            &body.role,
            body.canteen_ids,
            body.source_label,
        )
        .await?,
    ))
}

#[derive(Deserialize, Default)]
struct InviteBody {
    #[serde(default)]
    max_uses: Option<i32>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    canteen_ids: Vec<Uuid>,
}

async fn create_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    body: Option<Json<InviteBody>>,
) -> Result<impl IntoResponse, AppError> {
    let (max_uses, role, canteen_ids) = body
        .map(|item| (item.0.max_uses, item.0.role.clone(), item.0.canteen_ids.clone()))
        .unwrap_or((None, None, Vec::new()));
    let created = admin::create_invite(&state, &user, max_uses, role, canteen_ids).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<InviteBody>,
) -> Result<Json<Value>, AppError> {
    let max_uses = body
        .max_uses
        .ok_or_else(|| AppError::bad("max_uses_invalid", "可使用次数至少为 1"))?;
    Ok(Json(
        admin::update_invite_max_uses(&state, &user, id, max_uses).await?,
    ))
}

async fn revoke_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::revoke_invite(&state, &user, id).await?))
}

async fn list_invites(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::list_invites(&state, &user).await?))
}

#[derive(Deserialize)]
struct AdminMealQuery {
    date: NaiveDate,
    canteen_id: Uuid,
}

async fn admin_meals(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<AdminMealQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::admin_list_meals(&state, &user, query.date, query.canteen_id).await?,
    ))
}

async fn admin_put_meal(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((account_id, date, slot)): Path<(Uuid, NaiveDate, String)>,
    Json(body): Json<MealBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        meal::admin_put_meal(
            &state,
            &user,
            account_id,
            date,
            &slot,
            body.ordering,
            body.canteen_id,
        )
        .await?,
    ))
}

async fn get_email(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::get_email_enabled(&state, &user).await?))
}

#[derive(Deserialize)]
struct EmailSetting {
    enabled: bool,
}

async fn put_email(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<EmailSetting>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        admin::set_email_enabled(&state, &user, body.enabled).await?,
    ))
}

async fn get_alert(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::get_score_alert(&state, &user).await?))
}

#[derive(Deserialize)]
struct AlertBody {
    mean_threshold: f64,
    min_samples: i64,
}

async fn put_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<AlertBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        admin::set_score_alert(&state, &user, body.mean_threshold, body.min_samples).await?,
    ))
}

async fn get_channels(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::list_channels(&state, &user).await?))
}

#[derive(Deserialize)]
struct ChannelsBody {
    channels: Vec<ChannelBody>,
}

#[derive(Deserialize)]
struct ChannelBody {
    id: Option<Uuid>,
    kind: String,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

async fn put_channels(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ChannelsBody>,
) -> Result<Json<Value>, AppError> {
    let channels = body
        .channels
        .into_iter()
        .map(|item| admin::ChannelInput {
            id: item.id,
            kind: item.kind,
            base_url: item.base_url,
            model: item.model,
            api_key: item.api_key,
            enabled: item.enabled,
        })
        .collect();
    Ok(Json(admin::put_channels(&state, &user, channels).await?))
}

#[derive(Deserialize)]
struct RecoverBody {
    username: String,
    action: String,
    password: Option<String>,
}

async fn get_app_signing(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(crate::ops::app_signing::status(&state, &user).await?))
}

async fn put_app_signing(
    State(state): State<AppState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let mut keystore = None;
    let mut keystore_password = None;
    let mut key_alias = None;
    let mut key_password = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::bad("signing_field_invalid", "无法读取上传"))?
    {
        match field.name() {
            Some("keystore") => {
                keystore = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|_| AppError::bad("signing_field_invalid", "无法读取上传"))?,
                )
            }
            Some("keystore_password") => {
                keystore_password = Some(field.text().await.map_err(|_| {
                    AppError::bad("signing_field_invalid", "无法读取上传")
                })?)
            }
            Some("key_alias") => {
                key_alias = Some(field.text().await.map_err(|_| {
                    AppError::bad("signing_field_invalid", "无法读取上传")
                })?)
            }
            Some("key_password") => {
                key_password = Some(field.text().await.map_err(|_| {
                    AppError::bad("signing_field_invalid", "无法读取上传")
                })?)
            }
            _ => {}
        }
    }
    let (Some(keystore), Some(keystore_password), Some(key_alias), Some(key_password)) =
        (keystore, keystore_password, key_alias, key_password)
    else {
        return Err(AppError::bad(
            "signing_field_required",
            "需要 keystore 文件、keystore 密码、别名、key 密码",
        ));
    };
    Ok(Json(
        crate::ops::app_signing::put_keystore(
            &state,
            &user,
            &keystore,
            &keystore_password,
            &key_alias,
            &key_password,
        )
        .await?,
    ))
}

async fn generate_app_signing(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        crate::ops::app_signing::generate_keystore(&state, &user).await?,
    ))
}

async fn sign_app_release(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        crate::ops::app_signing::sign_release(&state, &user).await?,
    ))
}

async fn recover(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<RecoverBody>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        admin::recover(&state, &user, &body.username, &body.action, body.password).await?,
    ))
}

async fn accounts(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::list_accounts(&state, &user).await?))
}

// 测试时钟：后端认钟（state.clock 即 test_clock），总管侧拨到 T0–T5 切片。
// 仅 DLZ_ALLOW_TEST_CLOCK=1 时可用，且只认 super_admin；否则 404/403。
// 这是时间缝，不是业务 API：不建用户、不报餐、不评分。
fn test_clock_allowed() -> Result<(), AppError> {
    if std::env::var("DLZ_ALLOW_TEST_CLOCK").as_deref() == Ok("1") {
        Ok(())
    } else {
        Err(AppError::not_found("test_clock_disabled"))
    }
}

async fn get_test_clock(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    test_clock_allowed()?;
    crate::rbac::authorize(&user.assignments, crate::rbac::Perm::SystemSuper, None)?;
    Ok(Json(
        json!({ "now": state.test_clock.get(), "overridden": state.test_clock.get().is_some() }),
    ))
}

#[derive(Deserialize)]
struct TestClockBody {
    now: Option<String>,
    #[serde(default)]
    clear: bool,
}

async fn set_test_clock(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<TestClockBody>,
) -> Result<Json<Value>, AppError> {
    test_clock_allowed()?;
    crate::rbac::authorize(&user.assignments, crate::rbac::Perm::SystemSuper, None)?;
    let next = if body.clear || body.now.as_deref() == Some("") {
        None
    } else if let Some(raw) = body.now {
        let parsed = chrono::DateTime::parse_from_rfc3339(&raw)
            .map_err(|_| AppError::bad("test_clock_time", "now 需要 RFC3339 时间"))?;
        Some(parsed.with_timezone(&chrono::Utc))
    } else {
        None
    };
    state.test_clock.set(next);
    Ok(Json(json!({ "now": state.test_clock.get() })))
}

async fn audit(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    Ok(Json(admin::list_audit(&state, &user).await?))
}

async fn export_zip(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<impl IntoResponse, AppError> {
    let bytes = admin::export_zip(&state, &user).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"duolinzao-export.zip\"",
            ),
        ],
        bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{example_config, AppConfig};
    use crate::state::{CapturingMailer, ManualClock, ScriptedBackend};
    use http_body_util::BodyExt;
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// 无 DB 的路由测试：AppState 只解析配置（pool 用 lazy，不真连）。
    fn app_with_static(dir: &std::path::Path) -> Router {
        build_app(dir, router)
    }

    fn admin_app_with_static(dir: &std::path::Path) -> Router {
        build_app(dir, admin_router)
    }

    fn build_app(dir: &std::path::Path, build: fn(AppState) -> Router) -> Router {
        // Windows Temp 路径含反斜杠，TOML 基本串会转义失败，统一换成正斜杠（Windows 亦可识别）。
        let dir_str = dir.to_str().unwrap().replace('\\', "/");
        let cfg = example_config().replace(
            "[server]",
            &format!("[server]\nstatic_dir = \"{dir_str}\""),
        );
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://duolinzao:duolinzao@127.0.0.1:1/none")
            .expect("lazy pool");
        let state = AppState::new(
            pool,
            AppConfig::parse(&cfg).expect("config"),
            Arc::new(ManualClock::new(chrono::Utc::now())),
            Arc::new(CapturingMailer::default()),
            Arc::new(ScriptedBackend::failing_then_script()),
        )
        .expect("state");
        build(state)
    }

    async fn get(app: Router, uri: &str) -> (StatusCode, String, Vec<u8>) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("user-agent", "phase1-test")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.expect("oneshot");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, content_type, bytes.to_vec())
    }

    /// `/.well-known/assetlinks.json` 必须自己直出 JSON：不能被 SPA fallback 吃成
    /// index.html（200 + text/html），否则 Android 认为站点与 App 无关联，App 内通行密钥失败。
    #[tokio::test]
    async fn well_known_assetlinks_is_json_not_spa() {
        let base = std::env::temp_dir().join(format!("dlz-well-known-{}", Uuid::new_v4()));
        std::fs::create_dir_all(base.join("app")).unwrap();
        std::fs::write(base.join("index.html"), "<html>spa</html>").unwrap();
        let app = app_with_static(&base);

        // 未签名（无 certSha256）：200 + application/json + 空数组。
        std::fs::write(base.join("app/version.json"), r#"{"versionCode":3}"#).unwrap();
        let (status, content_type, bytes) =
            get(app.clone(), "/.well-known/assetlinks.json").await;
        assert_eq!(status, StatusCode::OK, "必须 200");
        assert_eq!(content_type, "application/json", "必须走 JSON 路由");
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!([]),
            "未签名应回空数组"
        );

        // 签名发布后：指纹来自 version.json 的 certSha256，包名固定。
        std::fs::write(
            base.join("app/version.json"),
            r#"{"versionCode":3,"certSha256":"AB:CD:EF"}"#,
        )
        .unwrap();
        let (status, content_type, bytes) = get(app, "/.well-known/assetlinks.json").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/json");
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body[0]["target"]["package_name"], "com.duolinzao.app", "{body}");
        assert_eq!(
            body[0]["target"]["sha256_cert_fingerprints"][0], "AB:CD:EF",
            "{body}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `/readyz` DB 坏回 503：lazy pool 指向不可达地址，无需真库，必 503 + db_unavailable。
    /// 同时守住短超时：没有 2s 短路时这里会被 pool 的 acquire_timeout 拖住
    /// （默认 30s，CI 实测约 30s），readiness 就失去快失败的意义。
    #[tokio::test]
    async fn readyz_db_unavailable_returns_503() {
        let base = std::env::temp_dir().join(format!("dlz-readyz-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let app = admin_app_with_static(&base);
        let started = std::time::Instant::now();
        let (status, content_type, bytes) = get(app, "/readyz").await;
        let elapsed = started.elapsed();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "DB 坏应 503");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "DB 断必须按 2s 短超时快失败，实际等了 {elapsed:?}"
        );
        assert_eq!(content_type, "application/json");
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "db_unavailable", "{body}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 无真库时本地跳过；CI 不允许跳过——test job 的 DATABASE_URL 一定可用，
    /// 这里再 skip 就说明正路径又没被测到（旧版 CI 静默 skip 的教训），必须红。
    fn absent_db(reason: impl std::fmt::Display) {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI 环境 readyz 正路径不能 skip（{reason}）"
        );
        eprintln!("skip readyz_ok: {reason}");
    }

    /// `/readyz` 健康库回 200：有真库就真断言，本地无库才 skip。
    ///
    /// 连接串优先级 **DATABASE_URL → TEST_DATABASE_URL → 本机默认**：CI test job 里
    /// DATABASE_URL 指向 postgres 维护库（service 容器保证存在），而 TEST_DATABASE_URL
    /// 的 duolinzao_test 由集成测试创建——lib 单测先跑时它还不存在，旧优先级会让 200
    /// 正路径在 CI 静默 skip、等于没测。顺序改过来后 CI 每次都会真走到 200 断言。
    #[tokio::test]
    async fn readyz_ok_returns_200_when_db_healthy() {
        let (url, source) = std::env::var("DATABASE_URL")
            .map(|url| (url, "DATABASE_URL"))
            .or_else(|_| std::env::var("TEST_DATABASE_URL").map(|url| (url, "TEST_DATABASE_URL")))
            .unwrap_or_else(|_| {
                (
                    "postgres://duolinzao:duolinzao@127.0.0.1:5432/duolinzao".to_string(),
                    "默认连接串",
                )
            });
        let pool = match PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect(&url)
            .await
        {
            Ok(p) => p,
            Err(err) => {
                absent_db(format_args!("{source} 连不上: {err}"));
                return;
            }
        };
        if let Err(err) = sqlx::query("SELECT 1").fetch_one(&pool).await {
            absent_db(format_args!("{source} 上 SELECT 1 失败: {err}"));
            return;
        }
        let cfg = example_config();
        let state = AppState::new(
            pool,
            AppConfig::parse(&cfg).expect("config"),
            Arc::new(ManualClock::new(chrono::Utc::now())),
            Arc::new(CapturingMailer::default()),
            Arc::new(ScriptedBackend::failing_then_script()),
        )
        .expect("state");
        let app = admin_router(state);
        let request = Request::builder()
            .method("GET")
            .uri("/readyz")
            .header("user-agent", "phase1-test")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["status"], "ok", "{body}");
    }

    /// 公网路由不挂 `/readyz`；真实部署有 `static_dir/index.html`，所以公网 GET /readyz
    /// 会被 SPA fallback 吃掉：返回 **index.html 的 body + text/html**，不是 readyz JSON。
    /// 注意状态码是 404 而不是 200——tower-http 的 `ServeDir::not_found_service` 会把
    /// fallback 响应的 status 固定改成 404（文件不存在仍是 404 语义），但 body 是首页。
    /// 结论不变：公网那个响应跟就绪无关；真正的探针只在 `admin_bind`（缺省 127.0.0.1:8090）。
    #[tokio::test]
    async fn readyz_not_on_public_router() {
        let base = std::env::temp_dir().join(format!("dlz-readyz-pub-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("index.html"), "<html>spa</html>").unwrap();
        let app = app_with_static(&base);
        let (status, content_type, bytes) = get(app, "/readyz").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "公网 /readyz 没有路由，落 SPA fallback（not_found_service 固定 404）"
        );
        assert!(
            content_type.starts_with("text/html"),
            "必须是 SPA 首页而非 readyz JSON，实际 Content-Type={content_type}"
        );
        assert_eq!(
            bytes,
            b"<html>spa</html>".to_vec(),
            "必须是 index.html 原文（公网看不到 readyz JSON，也拿不到 DB 就绪信息）"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
