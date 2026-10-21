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
use crate::ops::{admin, catalog, meal, session};
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

fn session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new("dlz_session", token.to_string());
    cookie.set_http_only(true);
    cookie.set_path("/");
    cookie.set_same_site(SameSite::Lax);
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

/// 站点自托管 `/app/`（通用 APK 与 version.json，供应用内更新同源拉取）。
/// 文件来自 `{static_dir}/app/`；缺文件、未配目录或路径逃逸一律 404 JSON，
/// 绝不回退 index.html（否则 App 会把 HTML 当 version.json 解析失败）。
async fn serve_app_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "不存在"})),
        )
            .into_response()
    };
    let Some(dir) = state.config.static_dir.clone() else {
        return not_found();
    };
    if path.contains("..") || path.contains('\\') || path.starts_with('/') {
        return not_found();
    }
    let full = format!("{dir}/app/{path}");
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
    // version.json 不缓存；APK 可缓存一小时（文件名随版本走 version.json 的 url）。
    let cache = if full.ends_with("version.json") {
        "no-store"
    } else {
        "public, max-age=3600"
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
    password: String,
}

async fn register(
    State(state): State<AppState>,
    ip: ClientIp,
    ua: UserAgent,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, AppError> {
    // 注册成功即登录：直接带上会话 cookie，不用再登一次。
    let (created, issued) =
        session::register(&state, &body.username, &body.invite_code, &body.password, &ua.0, &ip.0)
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
    username: String,
}

async fn passkey_start(
    State(state): State<AppState>,
    Json(body): Json<PasskeyStart>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        session::passkey_auth_start(&state, &body.username).await?,
    ))
}

#[derive(Deserialize)]
struct PasskeyFinish {
    username: String,
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
        &body.username,
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
