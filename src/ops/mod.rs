pub mod admin;
pub mod catalog;
pub mod meal;
pub mod session;

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::audit::{write_audit, AuditEntry};
use crate::crypto::{random_token, sha256_hex};
use crate::domain::friendly_device;
use crate::error::AppError;
use crate::extract::CurrentUser;
use crate::rbac::AuthHit;
use crate::state::AppState;

pub const SESSION_DAYS: i64 = 30;
pub const MAGIC_LINK_MINUTES: i64 = 15;
pub const LOGIN_WINDOW_MINUTES: i64 = 15;
pub const LOGIN_MAX_FAILURES: i64 = 20;
pub const PASSKEY_CHALLENGE_MINUTES: i64 = 5;
pub const GENERATE_HORIZON_DAYS: i64 = 14;

pub struct IssuedSession {
    pub raw_token: String,
    pub session_id: Uuid,
    pub device_id: Uuid,
}

pub async fn issue_session(
    pool: &PgPool,
    now: DateTime<Utc>,
    account_id: Uuid,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    let friendly = friendly_device(user_agent);
    let mut tx = pool.begin().await?;
    let device_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO device (account_id, friendly_name, user_agent, ip, created_at, last_seen_at)
        VALUES ($1, $2, $3, $4, $5, $5)
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(&friendly)
    .bind(user_agent)
    .bind(ip)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let raw_token = random_token();
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO session (account_id, device_id, token_hash, created_at, expires_at)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(device_id)
    .bind(sha256_hex(&raw_token))
    .bind(now)
    .bind(now + Duration::days(SESSION_DAYS))
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(IssuedSession {
        raw_token,
        session_id,
        device_id,
    })
}

pub async fn audit(
    state: &AppState,
    actor_id: Option<Uuid>,
    actor_username: Option<String>,
    hit: Option<&AuthHit>,
    action: &str,
    object_type: Option<&str>,
    object_id: Option<String>,
    summary: Value,
    result: &str,
) -> Result<(), AppError> {
    write_audit(
        &state.pool,
        AuditEntry {
            at: state.clock.now(),
            actor_id,
            actor_username,
            role: hit.map(|item| item.role.clone()),
            scope: hit
                .map(|item| item.scope.clone())
                .unwrap_or_else(|| "self".into()),
            action: action.into(),
            object_type: object_type.map(str::to_string),
            object_id,
            summary,
            result: result.into(),
        },
    )
    .await
}

pub async fn audit_user(
    state: &AppState,
    user: &CurrentUser,
    hit: &AuthHit,
    action: &str,
    object_type: Option<&str>,
    object_id: Option<String>,
    summary: Value,
) -> Result<(), AppError> {
    audit(
        state,
        Some(user.account_id),
        Some(user.username.clone()),
        Some(hit),
        action,
        object_type,
        object_id,
        summary,
        "ok",
    )
    .await
}

pub async fn email_enabled(state: &AppState) -> Result<bool, AppError> {
    let value: Option<Value> =
        sqlx::query_scalar("SELECT value FROM app_setting WHERE key = 'email_enabled'")
            .fetch_optional(&state.pool)
            .await?;
    Ok(value
        .and_then(|item| item.as_bool())
        .unwrap_or(state.config.email_enabled))
}

pub async fn alert_config(state: &AppState) -> Result<(f64, i64), AppError> {
    let mean: Option<Value> =
        sqlx::query_scalar("SELECT value FROM app_setting WHERE key = 'mean_threshold'")
            .fetch_optional(&state.pool)
            .await?;
    let samples: Option<Value> =
        sqlx::query_scalar("SELECT value FROM app_setting WHERE key = 'min_samples'")
            .fetch_optional(&state.pool)
            .await?;
    let mean = mean
        .and_then(|item| item.as_f64())
        .unwrap_or(state.config.mean_threshold);
    let samples = samples
        .and_then(|item| item.as_i64())
        .unwrap_or(state.config.min_samples);
    Ok((mean, samples))
}

pub async fn put_setting(
    pool: &PgPool,
    key: &str,
    value: Value,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO app_setting (key, value, updated_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(key)
    .bind(value)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn too_many_failures(
    pool: &PgPool,
    key: &str,
    now: DateTime<Utc>,
) -> Result<bool, AppError> {
    let since = now - Duration::minutes(LOGIN_WINDOW_MINUTES);
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM login_attempt
        WHERE key = $1 AND success = FALSE AND created_at > $2
        "#,
    )
    .bind(key)
    .bind(since)
    .fetch_one(pool)
    .await?;
    Ok(count >= LOGIN_MAX_FAILURES)
}

pub async fn record_attempt(
    pool: &PgPool,
    key: &str,
    success: bool,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO login_attempt (key, success, created_at) VALUES ($1, $2, $3)")
        .bind(key)
        .bind(success)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

/// 密码/TOTP/Passkey 共用的失败计数窗：用户键与 IP 键任一超限即 429。
pub async fn check_login_rate(
    pool: &PgPool,
    username: &str,
    ip: &str,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    if too_many_failures(pool, &format!("user:{username}"), now).await?
        || too_many_failures(pool, &format!("ip:{ip}"), now).await?
    {
        return Err(AppError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "login_rate_limited",
            "登录失败次数过多",
        ));
    }
    Ok(())
}

/// 成功/失败都要记两条（用户+IP），与密码登录一致。
pub async fn record_login(
    pool: &PgPool,
    username: &str,
    ip: &str,
    ok: bool,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    record_attempt(pool, &format!("user:{username}"), ok, now).await?;
    record_attempt(pool, &format!("ip:{ip}"), ok, now).await?;
    Ok(())
}

pub fn login_body(account_id: Uuid, username: &str) -> Value {
    json!({
        "account_id": account_id,
        "username": username,
    })
}

pub fn ensure_password(password: &str) -> Result<(), AppError> {
    if password.chars().count() < 8 {
        return Err(AppError::bad("password_weak", "密码至少 8 个字符"));
    }
    Ok(())
}

pub fn ensure_username(username: &str) -> Result<String, AppError> {
    let username = username.trim().to_string();
    if username.is_empty()
        || username.chars().count() > 64
        || username.chars().any(char::is_whitespace)
    {
        return Err(AppError::bad(
            "username_invalid",
            "账号需要是不含空白的字符串",
        ));
    }
    Ok(username)
}

/// 空字符串清空显示名。非空则去掉首尾空白，最长 64 个字符，拒绝控制字符。
pub fn normalize_display_name(raw: &str) -> Result<Option<String>, AppError> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(AppError::bad(
            "display_name_invalid",
            "显示名需要是不超过 64 个字符、不含控制字符的文本",
        ));
    }
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::normalize_display_name;

    #[test]
    fn display_name_trims_and_rejects_controls() {
        assert_eq!(normalize_display_name("  ").unwrap(), None);
        assert_eq!(
            normalize_display_name("  灶上的人  ").unwrap().as_deref(),
            Some("灶上的人")
        );
        assert!(normalize_display_name("a\nb").is_err());
        assert!(normalize_display_name(&"名".repeat(65)).is_err());
        assert_eq!(
            normalize_display_name(&"名".repeat(64))
                .unwrap()
                .unwrap()
                .chars()
                .count(),
            64
        );
    }
}
