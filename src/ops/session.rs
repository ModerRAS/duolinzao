use chrono::{Duration, Utc};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;
use webauthn_rs::prelude::{
    AuthenticationResult, DiscoverableKey, Passkey, PublicKeyCredential,
    RegisterPublicKeyCredential,
};

use super::{
    audit, audit_user, check_login_rate, email_enabled, ensure_password, ensure_username,
    issue_session, login_body, normalize_display_name, record_attempt, record_login,
    too_many_failures,
    IssuedSession, EMAIL_OTP_LOCK_MINUTES, EMAIL_OTP_MAX_FAILURES, EMAIL_OTP_MINUTES,
    EMAIL_OTP_RESEND_SECONDS, EMAIL_OTP_STEP_SECONDS, PASSKEY_CHALLENGE_MINUTES,
};
use crate::crypto::{
    decrypt, encrypt, hash_password, random_bytes, random_token, sha256_hex, verify_password,
};
use crate::domain::friendly_device;
use crate::error::AppError;
use crate::extract::{load_assignments, CurrentUser};
use crate::rbac::{authorize, Perm};
use crate::state::{AppState, Challenge};

pub async fn bootstrap(state: &AppState) -> Result<(), AppError> {
    let _ = tokio::fs::create_dir_all(&state.config.dish_image_dir).await;
    let now = state.clock.now();
    // 配置文件只提供初始值。总管理在运行中改过的开关和阈值，重启后保留。
    if sqlx::query_scalar::<_, Value>("SELECT value FROM app_setting WHERE key = 'email_enabled'")
        .fetch_optional(&state.pool)
        .await?
        .is_none()
    {
        super::put_setting(
            &state.pool,
            "email_enabled",
            json!(state.config.email_enabled),
            now,
        )
        .await?;
    }
    if sqlx::query_scalar::<_, Value>("SELECT value FROM app_setting WHERE key = 'mean_threshold'")
        .fetch_optional(&state.pool)
        .await?
        .is_none()
    {
        super::put_setting(
            &state.pool,
            "mean_threshold",
            json!(state.config.mean_threshold),
            now,
        )
        .await?;
    }
    if sqlx::query_scalar::<_, Value>("SELECT value FROM app_setting WHERE key = 'min_samples'")
        .fetch_optional(&state.pool)
        .await?
        .is_none()
    {
        super::put_setting(
            &state.pool,
            "min_samples",
            json!(state.config.min_samples),
            now,
        )
        .await?;
    }

    let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM account WHERE username = $1")
        .bind(&state.config.super_username)
        .fetch_optional(&state.pool)
        .await?;
    let account_id = if let Some(id) = existing {
        id
    } else {
        let password_hash = hash_password(&state.config.super_password).await?;
        sqlx::query_scalar(
            r#"
            INSERT INTO account (username, password_hash, created_at)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
        )
        .bind(&state.config.super_username)
        .bind(password_hash)
        .bind(now)
        .fetch_one(&state.pool)
        .await?
    };
    let has_role: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM role_assignment WHERE account_id = $1 AND role = 'super_admin')",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await?;
    if !has_role {
        sqlx::query(
            r#"
            INSERT INTO role_assignment (account_id, role, canteen_ids, created_at)
            VALUES ($1, 'super_admin', '{}', $2)
            "#,
        )
        .bind(account_id)
        .bind(now)
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

pub async fn methods(state: &AppState) -> Result<Value, AppError> {
    let mut methods = vec!["passkey", "totp"];
    let enabled = email_enabled(state).await?;
    if enabled {
        methods.push("email_otp");
    }
    methods.push("password");
    Ok(json!({
        "methods": methods,
        "email_enabled": enabled,
    }))
}

/// 注册成功即登录：事务提交后签发与密码登录相同的会话（不走 `login_approval` 新设备确认）。
/// 审计保留 register 一条并在 summary 标明已发会话，另记 method=`register` 的登录审计。
pub async fn register(
    state: &AppState,
    username: &str,
    invite_code: &str,
    password: &str,
    user_agent: &str,
    ip: &str,
) -> Result<(Value, IssuedSession), AppError> {
    let username = ensure_username(username)?;
    ensure_password(password)?;
    if invite_code.trim().is_empty() {
        return Err(AppError::bad("invite_invalid", "邀请码无效"));
    }
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    let invite = sqlx::query(
        r#"
        SELECT id, max_uses, used_count, revoked_at, role, canteen_ids FROM invite_code
        WHERE code_hash = $1
        FOR UPDATE
        "#,
    )
    .bind(sha256_hex(invite_code.trim()))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::bad("invite_invalid", "邀请码无效"))?;
    if invite
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at")
        .is_some()
    {
        return Err(AppError::bad("invite_invalid", "邀请码已作废"));
    }
    if invite.get::<i32, _>("used_count") >= invite.get::<i32, _>("max_uses") {
        return Err(AppError::bad("invite_invalid", "邀请码已用完"));
    }
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM account WHERE username = $1)")
            .bind(&username)
            .fetch_one(&mut *tx)
            .await?;
    if taken {
        return Err(AppError::conflict("username_taken", "账号已被占用"));
    }
    let password_hash = hash_password(password).await?;
    let account_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO account (username, password_hash, created_at)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(&username)
    .bind(password_hash)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    // 邀请码自带角色与范围：旧码默认为 employee + 空范围。
    let invite_role: String = invite.get("role");
    let invite_canteens: Vec<Uuid> = invite.get("canteen_ids");
    sqlx::query(
        r#"
        INSERT INTO role_assignment (account_id, role, canteen_ids, created_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(account_id)
    .bind(&invite_role)
    .bind(&invite_canteens)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    // 对齐 assign_role：食堂身份同步写入 canteen_staff。
    if invite_role == "canteen" {
        for id in &invite_canteens {
            sqlx::query("INSERT INTO canteen_staff (account_id, canteen_id) VALUES ($1, $2)")
                .bind(account_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    // used_by/used_at 只保留末次使用；次数以 used_count 为准。
    sqlx::query(
        "UPDATE invite_code SET used_by = $1, used_at = $2, used_count = used_count + 1 WHERE id = $3",
    )
    .bind(account_id)
    .bind(now)
    .bind(invite.get::<Uuid, _>("id"))
    .execute(&mut *tx)
    .await?;
    crate::audit::write_audit(
        &mut *tx,
        crate::audit::AuditEntry {
            at: now,
            actor_id: Some(account_id),
            actor_username: Some(username.clone()),
            role: Some(invite_role.clone()),
            scope: "self".into(),
            action: "register".into(),
            object_type: Some("account".into()),
            object_id: Some(account_id.to_string()),
            summary: json!({"username": username, "auto_login": true}),
            result: "ok".into(),
        },
    )
    .await?;
    tx.commit().await?;
    // 注册自动登录：与密码登录相同的会话，不进新设备确认流。
    let issued = issue_session(&state.pool, now, account_id, user_agent, ip).await?;
    audit(
        state,
        Some(account_id),
        Some(username.clone()),
        None,
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "register"}),
        "ok",
    )
    .await?;
    Ok((
        json!({"account_id": account_id, "username": username, "ok": true}),
        issued,
    ))
}

pub async fn login_password(
    state: &AppState,
    username: &str,
    password: &str,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    let now = state.clock.now();
    check_login_rate(&state.pool, username, ip, now).await?;
    let row = sqlx::query("SELECT id, username, password_hash FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let ok = if let Some(hash) = row
        .as_ref()
        .and_then(|item| item.get::<Option<String>, _>("password_hash"))
    {
        verify_password(password, &hash).await?
    } else {
        false
    };
    record_login(&state.pool, username, ip, ok, now).await?;
    if !ok {
        let _ = audit(
            state,
            None,
            Some(username.to_string()),
            None,
            "login",
            Some("session"),
            None,
            json!({"method": "password"}),
            "denied",
        )
        .await;
        return Err(AppError::bad("login_failed", "账号或密码不正确"));
    }
    let row = row.unwrap();
    let account_id: Uuid = row.get("id");
    let username: String = row.get("username");
    let issued = issue_session(&state.pool, now, account_id, user_agent, ip).await?;
    let assignments = load_assignments(&state.pool, account_id).await?;
    let hit = authorize(&assignments, Perm::DeviceApprove, None).ok();
    audit(
        state,
        Some(account_id),
        Some(username),
        hit.as_ref(),
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "password"}),
        "ok",
    )
    .await?;
    Ok(issued)
}

pub async fn me(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let row = sqlx::query(
        r#"
        SELECT a.email, a.email_verified, a.display_name,
               EXISTS(
                   SELECT 1 FROM totp_credential t
                   WHERE t.account_id = a.id AND t.confirmed
               ) AS totp
        FROM account a
        WHERE a.id = $1
        "#,
    )
    .bind(user.account_id)
    .fetch_one(&state.pool)
    .await?;
    let roles = user
        .assignments
        .iter()
        .map(|item| {
            json!({
                "role": item.role,
                "canteen_ids": item.canteen_ids,
                "source_label": item.source_label,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "id": user.account_id,
        "username": user.username,
        "display_name": row.get::<Option<String>, _>("display_name"),
        "email": row.get::<Option<String>, _>("email"),
        "email_verified": row.get::<bool, _>("email_verified"),
        "totp": row.get::<bool, _>("totp"),
        "roles": roles,
        "permissions": crate::rbac::permission_codes(&user.assignments),
        "email_enabled": email_enabled(state).await?,
    }))
}

pub async fn update_display_name(
    state: &AppState,
    user: &CurrentUser,
    raw: &str,
) -> Result<Value, AppError> {
    let display_name = normalize_display_name(raw)?;
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE account SET display_name = $1 WHERE id = $2")
        .bind(&display_name)
        .bind(user.account_id)
        .execute(&mut *tx)
        .await?;
    crate::audit::write_audit(
        &mut *tx,
        crate::audit::AuditEntry {
            at: now,
            actor_id: Some(user.account_id),
            actor_username: Some(user.username.clone()),
            role: Some(hit.role.clone()),
            scope: hit.scope.clone(),
            action: "display_name_update".into(),
            object_type: Some("account".into()),
            object_id: Some(user.account_id.to_string()),
            summary: json!({"display_name": display_name}),
            result: "ok".into(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"display_name": display_name}))
}

/// 自助改密：校验当前密码，保留当前会话，吊销该账号其它会话，并作废未消费的邮箱 OTP。
/// 总管找回仍走 `revoke_all_sessions`，连当前会话一起吊销。
pub async fn change_password(
    state: &AppState,
    user: &CurrentUser,
    current_password: &str,
    new_password: &str,
    ip: &str,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    let now = state.clock.now();
    check_login_rate(&state.pool, &user.username, ip, now).await?;
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query("SELECT password_hash FROM account WHERE id = $1 FOR UPDATE")
        .bind(user.account_id)
        .fetch_one(&mut *tx)
        .await?;
    let Some(hash) = row.get::<Option<String>, _>("password_hash") else {
        return Err(AppError::bad(
            "password_missing",
            "这个账号还没有密码，请联系总管找回",
        ));
    };
    if !verify_password(current_password, &hash).await? {
        drop(tx);
        record_login(&state.pool, &user.username, ip, false, now).await?;
        return Err(AppError::bad("password_incorrect", "当前密码不正确"));
    }
    if current_password == new_password {
        return Err(AppError::bad("password_unchanged", "新密码与当前密码相同"));
    }
    ensure_password(new_password)?;
    let new_hash = hash_password(new_password).await?;
    sqlx::query("UPDATE account SET password_hash = $1 WHERE id = $2")
        .bind(new_hash)
        .bind(user.account_id)
        .execute(&mut *tx)
        .await?;
    let revoked = sqlx::query(
        r#"
        UPDATE session SET revoked_at = $1
        WHERE account_id = $2 AND id <> $3 AND revoked_at IS NULL
        "#,
    )
    .bind(now)
    .bind(user.account_id)
    .bind(user.session_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query(
        "UPDATE magic_link SET consumed_at = $1 WHERE account_id = $2 AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(user.account_id)
    .execute(&mut *tx)
    .await?;
    // 待验证的邮箱 OTP 一并作废（magic_link 表只留历史行，不再写入新 token）。
    sqlx::query("DELETE FROM email_otp_challenge WHERE account_id = $1")
        .bind(user.account_id)
        .execute(&mut *tx)
        .await?;
    crate::audit::write_audit(
        &mut *tx,
        crate::audit::AuditEntry {
            at: now,
            actor_id: Some(user.account_id),
            actor_username: Some(user.username.clone()),
            role: Some(hit.role),
            scope: hit.scope,
            action: "password_change".into(),
            object_type: Some("account".into()),
            object_id: Some(user.account_id.to_string()),
            summary: json!({"revoked_other_sessions": revoked}),
            result: "ok".into(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(json!({
        "changed": true,
        "revoked_other_sessions": revoked,
    }))
}

pub async fn bind_email(
    state: &AppState,
    user: &CurrentUser,
    email: &str,
) -> Result<Value, AppError> {
    if !email_enabled(state).await? {
        return Err(AppError::bad("email_disabled", "邮箱能力已关闭"));
    }
    let email = email.trim().to_string();
    if !email.contains('@') || email.len() > 200 {
        return Err(AppError::bad("email_invalid", "邮箱格式不正确"));
    }
    let result = sqlx::query("UPDATE account SET email = $1, email_verified = FALSE WHERE id = $2")
        .bind(&email)
        .bind(user.account_id)
        .execute(&state.pool)
        .await;
    match result {
        Ok(_) => Ok(json!({"email": email})),
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            Err(AppError::conflict("email_taken", "邮箱已被占用"))
        }
        Err(err) => Err(err.into()),
    }
}

pub async fn totp_setup(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let secret = random_bytes(20);
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret.clone(),
        Some("多邻灶".into()),
        user.username.clone(),
    )
    .map_err(|_| AppError::internal())?;
    let url = totp.get_url();
    let base32 = totp.get_secret_base32();
    let packed = encrypt(&state.config.local_key, &secret)?;
    sqlx::query(
        r#"
        INSERT INTO totp_credential (account_id, secret_ciphertext, confirmed, created_at)
        VALUES ($1, $2, FALSE, $3)
        ON CONFLICT (account_id) DO UPDATE
        SET secret_ciphertext = EXCLUDED.secret_ciphertext, confirmed = FALSE, created_at = EXCLUDED.created_at
        "#,
    )
    .bind(user.account_id)
    .bind(packed)
    .bind(state.clock.now())
    .execute(&state.pool)
    .await?;
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    audit_user(
        state,
        user,
        &hit,
        "totp_setup",
        Some("totp"),
        Some(user.account_id.to_string()),
        json!({}),
    )
    .await?;
    Ok(json!({"secret": base32, "otpauth_url": url}))
}

pub async fn totp_confirm(
    state: &AppState,
    user: &CurrentUser,
    code: &str,
) -> Result<Value, AppError> {
    match check_totp(state, user.account_id, &user.username, code).await? {
        TotpOutcome::Invalid => Err(AppError::bad("totp_invalid", "验证码不正确")),
        TotpOutcome::Replayed => Err(AppError::bad("totp_reused", "验证码已使用过")),
        TotpOutcome::Valid(step) => {
            sqlx::query(
                "UPDATE totp_credential SET confirmed = TRUE, last_used_step = $2 WHERE account_id = $1",
            )
            .bind(user.account_id)
            .bind(step)
            .execute(&state.pool)
            .await?;
            Ok(json!({"confirmed": true}))
        }
    }
}

enum TotpOutcome {
    Invalid,
    Replayed,
    Valid(i64),
}

async fn check_totp(
    state: &AppState,
    account_id: Uuid,
    username: &str,
    code: &str,
) -> Result<TotpOutcome, AppError> {
    let row = sqlx::query(
        "SELECT secret_ciphertext, last_used_step FROM totp_credential WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Ok(TotpOutcome::Invalid);
    };
    let secret = decrypt(
        &state.config.local_key,
        &row.get::<Vec<u8>, _>("secret_ciphertext"),
    )?;
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some("多邻灶".into()),
        username.to_string(),
    )
    .map_err(|_| AppError::internal())?;
    let now = state.clock.now().timestamp().max(0) as u64;
    if !totp.check(code.trim(), now) {
        return Ok(TotpOutcome::Invalid);
    }
    // 一次性：记下已用时间窗，同窗再验拒绝。
    let step = (now / 30) as i64;
    if row.get::<Option<i64>, _>("last_used_step") == Some(step) {
        return Ok(TotpOutcome::Replayed);
    }
    Ok(TotpOutcome::Valid(step))
}

pub async fn login_totp(
    state: &AppState,
    username: &str,
    code: &str,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    let now = state.clock.now();
    // 与密码登录共用同一失败计数窗；失败也要落库，避免暴力枚举验证码。
    check_login_rate(&state.pool, username, ip, now).await?;
    let row = sqlx::query(
        r#"
        SELECT a.id, a.username, t.confirmed
        FROM account a
        JOIN totp_credential t ON t.account_id = a.id
        WHERE a.username = $1
        "#,
    )
    .bind(username)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        record_login(&state.pool, username, ip, false, now).await?;
        return Err(AppError::bad("totp_invalid", "验证码不正确"));
    };
    if !row.get::<bool, _>("confirmed") {
        record_login(&state.pool, username, ip, false, now).await?;
        return Err(AppError::bad("totp_invalid", "验证码不正确"));
    }
    let account_id: Uuid = row.get("id");
    let username: String = row.get("username");
    let step = match check_totp(state, account_id, &username, code).await? {
        TotpOutcome::Invalid => {
            record_login(&state.pool, &username, ip, false, now).await?;
            return Err(AppError::bad("totp_invalid", "验证码不正确"));
        }
        TotpOutcome::Replayed => {
            record_login(&state.pool, &username, ip, false, now).await?;
            return Err(AppError::bad("totp_reused", "验证码已使用过"));
        }
        TotpOutcome::Valid(step) => step,
    };
    sqlx::query("UPDATE totp_credential SET last_used_step = $1 WHERE account_id = $2")
        .bind(step)
        .bind(account_id)
        .execute(&state.pool)
        .await?;
    record_login(&state.pool, &username, ip, true, now).await?;
    let issued = issue_session(&state.pool, now, account_id, user_agent, ip).await?;
    audit(
        state,
        Some(account_id),
        Some(username),
        None,
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "totp"}),
        "ok",
    )
    .await?;
    Ok(issued)
}

pub async fn request_email_otp(state: &AppState, username: &str) -> Result<Value, AppError> {
    if !email_enabled(state).await? {
        return Err(AppError::bad("email_disabled", "邮箱能力已关闭"));
    }
    let accepted = || Ok(json!({"accepted": true}));
    let row = sqlx::query("SELECT id, email FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return accepted();
    };
    let account_id: Uuid = row.get("id");
    // 防枚举：不存在、未绑邮箱都与正常请求返回一致，且不发信。
    let Some(email) = row.get::<Option<String>, _>("email") else {
        return accepted();
    };
    if email.trim().is_empty() {
        return accepted();
    }
    let now = state.clock.now();
    let challenge = sqlx::query(
        "SELECT last_sent_at, fail_count, locked_until FROM email_otp_challenge WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(item) = challenge.as_ref() {
        if let Some(locked) = item.get::<Option<chrono::DateTime<Utc>>, _>("locked_until") {
            if locked > now {
                return Err(AppError::new(
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    "otp_locked",
                    "验证码尝试过多，请稍后再试",
                ));
            }
        }
    }
    if let Some(item) = challenge.as_ref() {
        let last = item.get::<chrono::DateTime<Utc>, _>("last_sent_at");
        if now.signed_duration_since(last).num_seconds() < EMAIL_OTP_RESEND_SECONDS {
            return Err(AppError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "otp_resend_too_soon",
                "验证码发送太频繁，请稍后再试",
            ));
        }
    }
    // 每次 request 签发独立 secret（10 分钟窗），不是用户的认证器长期 secret。
    let secret = random_bytes(20);
    let step = now.timestamp().max(0) / EMAIL_OTP_STEP_SECONDS;
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        EMAIL_OTP_STEP_SECONDS as u64,
        secret.clone(),
        Some("多邻灶".into()),
        username.to_string(),
    )
    .map_err(|_| AppError::internal())?;
    let code = totp.generate((step * EMAIL_OTP_STEP_SECONDS) as u64);
    let packed = encrypt(&state.config.local_key, &secret)?;
    // 失败计数跨 request 保留（只在成功时清零），否则重发即可绕开锁定。
    sqlx::query(
        r#"
        INSERT INTO email_otp_challenge (
            account_id, secret_ciphertext, step, expires_at, last_sent_at, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $5)
        ON CONFLICT (account_id) DO UPDATE SET
            secret_ciphertext = EXCLUDED.secret_ciphertext,
            step = EXCLUDED.step,
            expires_at = EXCLUDED.expires_at,
            last_sent_at = EXCLUDED.last_sent_at,
            used_step = NULL
        "#,
    )
    .bind(account_id)
    .bind(packed)
    .bind(step)
    .bind(now + Duration::minutes(EMAIL_OTP_MINUTES))
    .bind(now)
    .execute(&state.pool)
    .await?;
    let body = format!(
        "多邻灶登录验证码：{code}\n10 分钟内有效，请勿转发。\n如果不是本人操作请忽略。"
    );
    state
        .mailer
        .send(&email, "多邻灶登录验证码", &body)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "email otp mail failed");
            AppError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "mail_failed",
                "邮件发送失败",
            )
        })?;
    accepted()
}

/// 邮箱 OTP 校验：成功后持久化已用时间窗，同窗再验拒绝；顺手把邮箱标为已验证。
pub async fn verify_email_otp(
    state: &AppState,
    username: &str,
    code: &str,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    if !email_enabled(state).await? {
        return Err(AppError::bad("email_disabled", "邮箱能力已关闭"));
    }
    let now = state.clock.now();
    // 与密码/TOTP 登录共用同一失败计数窗；失败也要落库，避免暴力枚举验证码。
    check_login_rate(&state.pool, username, ip, now).await?;
    let fail = async |username: &str, ip: &str| {
        let _ = record_login(&state.pool, username, ip, false, state.clock.now()).await;
    };
    let invalid = || AppError::bad("otp_invalid", "验证码不正确");
    let row = sqlx::query("SELECT id, username FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        fail(username, ip).await;
        return Err(invalid());
    };
    let account_id: Uuid = row.get("id");
    let username: String = row.get("username");
    let challenge =
        sqlx::query("SELECT * FROM email_otp_challenge WHERE account_id = $1")
            .bind(account_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some(item) = challenge else {
        fail(&username, ip).await;
        return Err(invalid());
    };
    if let Some(locked) = item.get::<Option<chrono::DateTime<Utc>>, _>("locked_until") {
        if locked > now {
            fail(&username, ip).await;
            return Err(AppError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "otp_locked",
                "验证码尝试过多，请稍后再试",
            ));
        }
    }
    let step: i64 = item.get("step");
    // 一次性：同窗再验拒绝（成功即记 used_step，新 request 会清掉它）。
    if item.get::<Option<i64>, _>("used_step") == Some(step) {
        fail(&username, ip).await;
        return Err(AppError::bad("otp_reused", "验证码已使用过"));
    }
    if item.get::<chrono::DateTime<Utc>, _>("expires_at") <= now {
        bump_otp_failure(state, account_id, &username, ip, now).await?;
        return Err(AppError::bad("otp_expired", "验证码已过期，请重新获取"));
    }
    let secret = decrypt(
        &state.config.local_key,
        &item.get::<Vec<u8>, _>("secret_ciphertext"),
    )?;
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        EMAIL_OTP_STEP_SECONDS as u64,
        secret,
        Some("多邻灶".into()),
        username.clone(),
    )
    .map_err(|_| AppError::internal())?;
    if totp.generate((step * EMAIL_OTP_STEP_SECONDS) as u64) != code.trim() {
        bump_otp_failure(state, account_id, &username, ip, now).await?;
        return Err(invalid());
    }
    sqlx::query(
        "UPDATE email_otp_challenge SET used_step = $1, fail_count = 0 WHERE account_id = $2",
    )
    .bind(step)
    .bind(account_id)
    .execute(&state.pool)
    .await?;
    sqlx::query("UPDATE account SET email_verified = TRUE WHERE id = $1")
        .bind(account_id)
        .execute(&state.pool)
        .await?;
    record_login(&state.pool, &username, ip, true, now).await?;
    let issued = issue_session(&state.pool, now, account_id, user_agent, ip).await?;
    audit(
        state,
        Some(account_id),
        Some(username),
        None,
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "email_otp"}),
        "ok",
    )
    .await?;
    Ok(issued)
}

/// 邮箱 OTP 失败计数：连续 5 次失败锁定该账号 15 分钟；同时记登录失败窗。
async fn bump_otp_failure(
    state: &AppState,
    account_id: Uuid,
    username: &str,
    ip: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    record_login(&state.pool, username, ip, false, now).await?;
    sqlx::query(
        r#"
        UPDATE email_otp_challenge
        SET fail_count = fail_count + 1,
            locked_until = CASE
                WHEN fail_count + 1 >= $2 THEN $3
                ELSE locked_until
            END
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .bind(EMAIL_OTP_MAX_FAILURES as i32)
    .bind(now + Duration::minutes(EMAIL_OTP_LOCK_MINUTES))
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn passkey_register_start(
    state: &AppState,
    user: &CurrentUser,
) -> Result<Value, AppError> {
    let exclude = passkey_ids(state, user.account_id).await?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM account WHERE id = $1")
            .bind(user.account_id)
            .fetch_one(&state.pool)
            .await?;
    let label = stored
        .filter(|item| !item.trim().is_empty())
        .unwrap_or_else(|| user.username.clone());
    let (challenge, reg_state) = state
        .webauthn
        .start_passkey_registration(user.account_id, &user.username, &label, Some(exclude))
        .map_err(|err| {
            tracing::warn!(error = ?err, "passkey start");
            AppError::bad("passkey_failed", "无法开始绑定 Passkey")
        })?;
    let id = Uuid::new_v4();
    state
        .challenges
        .insert(
            id,
            Challenge::Register {
                account_id: user.account_id,
                state: reg_state,
                expires_at: state.clock.now() + Duration::minutes(PASSKEY_CHALLENGE_MINUTES),
            },
        )
        .await;
    Ok(json!({"challenge_id": id, "publicKey": challenge}))
}

pub async fn passkey_register_finish(
    state: &AppState,
    user: &CurrentUser,
    challenge_id: Uuid,
    credential: RegisterPublicKeyCredential,
    friendly_name: Option<String>,
) -> Result<Value, AppError> {
    let stored = state
        .challenges
        .take(challenge_id, state.clock.now())
        .await
        .ok_or_else(|| AppError::bad("passkey_failed", "绑定已过期"))?;
    let Challenge::Register {
        account_id,
        state: reg_state,
        ..
    } = stored
    else {
        return Err(AppError::bad("passkey_failed", "绑定状态不匹配"));
    };
    if account_id != user.account_id {
        return Err(AppError::forbidden());
    }
    let passkey = state
        .webauthn
        .finish_passkey_registration(&credential, &reg_state)
        .map_err(|_| AppError::bad("passkey_failed", "Passkey 校验失败"))?;
    let id = store_passkey(state, user.account_id, &passkey, friendly_name).await?;
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    audit_user(
        state,
        user,
        &hit,
        "passkey_bind",
        Some("passkey"),
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(json!({"id": id}))
}

async fn store_passkey(
    state: &AppState,
    account_id: Uuid,
    passkey: &Passkey,
    friendly_name: Option<String>,
) -> Result<Uuid, AppError> {
    let cred_id = passkey.cred_id().as_ref().to_vec();
    let doc = serde_json::to_value(passkey).map_err(|_| AppError::internal())?;
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO passkey (account_id, credential_id, passkey_json, friendly_name, created_at)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(cred_id)
    .bind(doc)
    .bind(friendly_name)
    .bind(state.clock.now())
    .fetch_one(&state.pool)
    .await?;
    Ok(id)
}

async fn passkey_ids(
    state: &AppState,
    account_id: Uuid,
) -> Result<Vec<webauthn_rs::prelude::CredentialID>, AppError> {
    let rows = load_passkeys(state, account_id).await?;
    Ok(rows
        .into_iter()
        .map(|item| item.cred_id().clone())
        .collect())
}

fn load_passkey_value(value: Value) -> Option<Passkey> {
    serde_json::from_value(value).ok()
}

async fn load_passkeys(state: &AppState, account_id: Uuid) -> Result<Vec<Passkey>, AppError> {
    let rows = sqlx::query(
        "SELECT passkey_json FROM passkey WHERE account_id = $1 AND revoked_at IS NULL",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| load_passkey_value(row.get("passkey_json")))
        .collect())
}

fn passkey_none() -> AppError {
    AppError::new(
        axum::http::StatusCode::NOT_FOUND,
        "passkey_none",
        "这个账号还没有 Passkey，请用密码登录",
    )
}

/// Passkey 登录 start：username 可选。缺省或空走 discoverable（免填账号，
/// 浏览器/系统弹账号选择器）；传了 username 则保留旧 allowCredentials 路径，兼容旧客户端。
pub async fn passkey_auth_start(
    state: &AppState,
    username: Option<&str>,
) -> Result<Value, AppError> {
    let username = username.map(str::trim).filter(|item| !item.is_empty());
    if username.is_none() {
        let (challenge, auth_state) = state
            .webauthn
            .start_discoverable_authentication()
            .map_err(|_| AppError::bad("passkey_failed", "无法开始 Passkey 登录"))?;
        let id = Uuid::new_v4();
        state
            .challenges
            .insert(
                id,
                Challenge::Discoverable {
                    state: auth_state,
                    expires_at: state.clock.now() + Duration::minutes(PASSKEY_CHALLENGE_MINUTES),
                },
            )
            .await;
        return Ok(json!({"challenge_id": id, "publicKey": challenge}));
    }
    let username = username.unwrap();
    let account_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let Some(account_id) = account_id else {
        return Err(passkey_none());
    };
    let creds = load_passkeys(state, account_id).await?;
    if creds.is_empty() {
        return Err(passkey_none());
    }
    let (challenge, auth_state) = state
        .webauthn
        .start_passkey_authentication(&creds)
        .map_err(|_| AppError::bad("passkey_failed", "无法开始 Passkey 登录"))?;
    let id = Uuid::new_v4();
    state
        .challenges
        .insert(
            id,
            Challenge::Authenticate {
                state: auth_state,
                expires_at: state.clock.now() + Duration::minutes(PASSKEY_CHALLENGE_MINUTES),
            },
        )
        .await;
    Ok(json!({"challenge_id": id, "publicKey": challenge}))
}

/// Passkey 登录 finish：username 可选。usernameless 时以 credential 的
/// userHandle 为准，不依赖 body.username。
pub async fn passkey_auth_finish(
    state: &AppState,
    username: Option<&str>,
    challenge_id: Uuid,
    credential: PublicKeyCredential,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    let username = username.map(str::trim).filter(|item| !item.is_empty());
    match username {
        Some(name) => {
            passkey_auth_finish_named(state, name, challenge_id, credential, user_agent, ip).await
        }
        None => {
            passkey_auth_finish_discoverable(state, challenge_id, credential, user_agent, ip).await
        }
    }
}

async fn passkey_auth_finish_named(
    state: &AppState,
    username: &str,
    challenge_id: Uuid,
    credential: PublicKeyCredential,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    let now = state.clock.now();
    // 与密码登录共用同一失败计数窗；每次失败都落库。
    check_login_rate(&state.pool, username, ip, now).await?;
    let fail = async |username: &str, ip: &str| {
        let _ = record_login(&state.pool, username, ip, false, state.clock.now()).await;
    };
    let stored = state
        .challenges
        .take(challenge_id, now)
        .await
        .ok_or_else(|| AppError::bad("passkey_failed", "登录已过期"));
    let stored = match stored {
        Ok(item) => item,
        Err(err) => {
            fail(username, ip).await;
            return Err(err);
        }
    };
    let Challenge::Authenticate {
        state: auth_state, ..
    } = stored
    else {
        fail(username, ip).await;
        return Err(AppError::bad("passkey_failed", "登录状态不匹配"));
    };
    let result = match state
        .webauthn
        .finish_passkey_authentication(&credential, &auth_state)
    {
        Ok(item) => item,
        Err(_) => {
            fail(username, ip).await;
            return Err(AppError::bad("passkey_failed", "Passkey 校验失败"));
        }
    };
    let account = sqlx::query("SELECT id, username FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let Some(account) = account else {
        fail(username, ip).await;
        return Err(AppError::bad("passkey_failed", "Passkey 校验失败"));
    };
    let account_id: Uuid = account.get("id");
    if !touch_passkey_counter(state, account_id, &result).await? {
        fail(username, ip).await;
        return Err(AppError::bad("passkey_failed", "Passkey 不属于该账号"));
    }
    record_login(&state.pool, username, ip, true, now).await?;
    let issued = issue_session(&state.pool, state.clock.now(), account_id, user_agent, ip).await?;
    audit(
        state,
        Some(account_id),
        Some(account.get("username")),
        None,
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "passkey"}),
        "ok",
    )
    .await?;
    Ok(issued)
}

/// Usernameless 登录：账号从 credential 的 userHandle（= account_id）里拿，
/// 不依赖请求里的 username。
async fn passkey_auth_finish_discoverable(
    state: &AppState,
    challenge_id: Uuid,
    credential: PublicKeyCredential,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    let now = state.clock.now();
    // 还没拿到账号，先只看 IP 限流，避免无账号枚举打爆。
    if too_many_failures(&state.pool, &format!("ip:{ip}"), now).await? {
        return Err(AppError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "login_rate_limited",
            "登录失败次数过多",
        ));
    }
    // 尚无账号，只有 IP 可记账。
    let fail_ip = async {
        let _ = record_attempt(&state.pool, &format!("ip:{ip}"), false, state.clock.now()).await;
    };
    let stored = state.challenges.take(challenge_id, now).await;
    let Some(stored) = stored else {
        fail_ip.await;
        return Err(AppError::bad("passkey_failed", "登录已过期"));
    };
    let Challenge::Discoverable {
        state: auth_state, ..
    } = stored
    else {
        fail_ip.await;
        return Err(AppError::bad("passkey_failed", "登录状态不匹配"));
    };
    let account_id = match state
        .webauthn
        .identify_discoverable_authentication(&credential)
    {
        Ok((id, _)) => id,
        Err(_) => {
            fail_ip.await;
            return Err(AppError::bad("passkey_failed", "Passkey 校验失败"));
        }
    };
    let account = sqlx::query("SELECT id, username FROM account WHERE id = $1")
        .bind(account_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(account) = account else {
        fail_ip.await;
        return Err(AppError::bad("passkey_failed", "Passkey 校验失败"));
    };
    let username: String = account.get("username");
    // 拿到账号后与密码登录共用同一失败计数窗；每次失败都落库。
    check_login_rate(&state.pool, &username, ip, now).await?;
    let keys = load_passkeys(state, account_id).await?;
    if keys.is_empty() {
        record_login(&state.pool, &username, ip, false, now).await?;
        return Err(passkey_none());
    }
    let creds: Vec<DiscoverableKey> = keys.iter().map(DiscoverableKey::from).collect();
    let result = match state
        .webauthn
        .finish_discoverable_authentication(&credential, auth_state, &creds)
    {
        Ok(item) => item,
        Err(_) => {
            record_login(&state.pool, &username, ip, false, now).await?;
            return Err(AppError::bad("passkey_failed", "Passkey 校验失败"));
        }
    };
    if !touch_passkey_counter(state, account_id, &result).await? {
        record_login(&state.pool, &username, ip, false, now).await?;
        return Err(AppError::bad("passkey_failed", "Passkey 不属于该账号"));
    }
    record_login(&state.pool, &username, ip, true, now).await?;
    let issued = issue_session(&state.pool, state.clock.now(), account_id, user_agent, ip).await?;
    audit(
        state,
        Some(account_id),
        Some(username),
        None,
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "passkey"}),
        "ok",
    )
    .await?;
    Ok(issued)
}

/// 断言命中的 credential 属于该账号时更新 counter；返回是否命中。
async fn touch_passkey_counter(
    state: &AppState,
    account_id: Uuid,
    result: &AuthenticationResult,
) -> Result<bool, AppError> {
    let mut rows = sqlx::query(
        "SELECT id, passkey_json FROM passkey WHERE account_id = $1 AND revoked_at IS NULL",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await?;
    for row in &mut rows {
        let mut passkey: Passkey =
            serde_json::from_value(row.get("passkey_json")).map_err(|_| AppError::internal())?;
        if passkey.cred_id().as_ref() == result.cred_id().as_ref() {
            if passkey.update_credential(result) == Some(true) {
                let doc = serde_json::to_value(&passkey).map_err(|_| AppError::internal())?;
                sqlx::query("UPDATE passkey SET passkey_json = $1 WHERE id = $2")
                    .bind(doc)
                    .bind(row.get::<Uuid, _>("id"))
                    .execute(&state.pool)
                    .await?;
            }
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn list_devices(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let now = state.clock.now();
    let devices = sqlx::query(
        r#"
        SELECT d.id, d.friendly_name, d.user_agent, d.ip, d.created_at, d.last_seen_at,
               EXISTS(
                   SELECT 1 FROM session s
                   WHERE s.device_id = d.id AND s.revoked_at IS NULL AND s.expires_at > $2
               ) AS session_active
        FROM device d
        WHERE d.account_id = $1
        ORDER BY d.created_at DESC
        "#,
    )
    .bind(user.account_id)
    .bind(now)
    .fetch_all(&state.pool)
    .await?;
    let passkeys = sqlx::query(
        r#"
        SELECT id, friendly_name, created_at
        FROM passkey
        WHERE account_id = $1 AND revoked_at IS NULL
        ORDER BY created_at
        "#,
    )
    .bind(user.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "devices": devices.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "friendly_name": row.get::<String, _>("friendly_name"),
            "user_agent": row.get::<String, _>("user_agent"),
            "ip": row.get::<String, _>("ip"),
            "created_at": row.get::<chrono::DateTime<Utc>, _>("created_at"),
            "last_seen_at": row.get::<chrono::DateTime<Utc>, _>("last_seen_at"),
            "session_active": row.get::<bool, _>("session_active"),
        })).collect::<Vec<_>>(),
        "passkeys": passkeys.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "friendly_name": row.get::<Option<String>, _>("friendly_name"),
            "created_at": row.get::<chrono::DateTime<Utc>, _>("created_at"),
        })).collect::<Vec<_>>(),
    }))
}

pub async fn kick_device(
    state: &AppState,
    user: &CurrentUser,
    device_id: Uuid,
) -> Result<Value, AppError> {
    let owned: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM device WHERE id = $1 AND account_id = $2)")
            .bind(device_id)
            .bind(user.account_id)
            .fetch_one(&state.pool)
            .await?;
    if !owned {
        return Err(AppError::not_found("device_not_found"));
    }
    let now = state.clock.now();
    sqlx::query("UPDATE session SET revoked_at = $1 WHERE device_id = $2 AND revoked_at IS NULL")
        .bind(now)
        .bind(device_id)
        .execute(&state.pool)
        .await?;
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    audit_user(
        state,
        user,
        &hit,
        "device_kick",
        Some("device"),
        Some(device_id.to_string()),
        json!({}),
    )
    .await?;
    Ok(json!({"revoked": true}))
}

pub async fn unbind_passkey(
    state: &AppState,
    user: &CurrentUser,
    passkey_id: Uuid,
) -> Result<Value, AppError> {
    let now = state.clock.now();
    let updated = sqlx::query(
        r#"
        UPDATE passkey SET revoked_at = $1
        WHERE id = $2 AND account_id = $3 AND revoked_at IS NULL
        "#,
    )
    .bind(now)
    .bind(passkey_id)
    .bind(user.account_id)
    .execute(&state.pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::not_found("passkey_not_found"));
    }
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    audit_user(
        state,
        user,
        &hit,
        "passkey_unbind",
        Some("passkey"),
        Some(passkey_id.to_string()),
        json!({}),
    )
    .await?;
    Ok(json!({"revoked": true}))
}

pub async fn create_approval(
    state: &AppState,
    username: &str,
    user_agent: &str,
    ip: &str,
) -> Result<Value, AppError> {
    let now = state.clock.now();
    if too_many_failures(&state.pool, &format!("ip:{ip}"), now).await? {
        return Err(AppError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "login_rate_limited",
            "请求过多",
        ));
    }
    let account_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let Some(account_id) = account_id else {
        return Err(AppError::not_found("account_not_found"));
    };
    let poll = random_token();
    let friendly = friendly_device(user_agent);
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO login_approval (
            account_id, status, user_agent, friendly_name, ip, poll_token_hash, created_at
        )
        VALUES ($1, 'pending', $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(user_agent)
    .bind(&friendly)
    .bind(ip)
    .bind(sha256_hex(&poll))
    .bind(now)
    .fetch_one(&state.pool)
    .await?;
    Ok(json!({
        "id": id,
        "poll_token": poll,
        "status": "pending",
        "user_agent": user_agent,
        "ip": ip,
        "friendly_name": friendly,
    }))
}

pub async fn pending_approvals(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    let rows = sqlx::query(
        r#"
        SELECT id, user_agent, friendly_name, ip, created_at, status
        FROM login_approval
        WHERE account_id = $1 AND status = 'pending'
        ORDER BY created_at
        "#,
    )
    .bind(user.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "user_agent": row.get::<String, _>("user_agent"),
            "friendly_name": row.get::<String, _>("friendly_name"),
            "ip": row.get::<String, _>("ip"),
            "created_at": row.get::<chrono::DateTime<Utc>, _>("created_at"),
            "status": row.get::<String, _>("status"),
        })).collect::<Vec<_>>()
    }))
}

async fn decide(
    state: &AppState,
    user: &CurrentUser,
    approval_id: Uuid,
    allow: bool,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::DeviceApprove, None)?;
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        r#"
        SELECT account_id, status, user_agent, ip
        FROM login_approval
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(approval_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("approval_not_found"))?;
    if row.get::<Uuid, _>("account_id") != user.account_id {
        return Err(AppError::forbidden());
    }
    if row.get::<String, _>("status") != "pending" {
        return Err(AppError::conflict("approval_closed", "这条请求已经结束"));
    }
    let status = if allow { "allowed" } else { "denied" };
    let mut granted_session = None;
    let mut ciphertext = None;
    if allow {
        let issued = issue_session(
            &state.pool,
            now,
            user.account_id,
            &row.get::<String, _>("user_agent"),
            &row.get::<String, _>("ip"),
        )
        .await?;
        ciphertext = Some(encrypt(
            &state.config.local_key,
            issued.raw_token.as_bytes(),
        )?);
        granted_session = Some(issued.session_id);
    }
    let updated = sqlx::query(
        r#"
        UPDATE login_approval
        SET status = $1, decided_at = $2, decided_by = $3,
            granted_session_id = $4, granted_token_ciphertext = $5
        WHERE id = $6 AND status = 'pending'
        "#,
    )
    .bind(status)
    .bind(now)
    .bind(user.account_id)
    .bind(granted_session)
    .bind(ciphertext)
    .bind(approval_id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::conflict("approval_closed", "这条请求已经结束"));
    }
    let action = if allow {
        "login_approval_allow"
    } else {
        "login_approval_deny"
    };
    crate::audit::write_audit(
        &mut *tx,
        crate::audit::AuditEntry {
            at: now,
            actor_id: Some(user.account_id),
            actor_username: Some(user.username.clone()),
            role: Some(hit.role.clone()),
            scope: hit.scope.clone(),
            action: action.into(),
            object_type: Some("login_approval".into()),
            object_id: Some(approval_id.to_string()),
            summary: json!({"status": status}),
            result: "ok".into(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"status": status}))
}

pub async fn allow_approval(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
) -> Result<Value, AppError> {
    decide(state, user, id, true).await
}

pub async fn deny_approval(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
) -> Result<Value, AppError> {
    decide(state, user, id, false).await
}

pub async fn poll_approval(
    state: &AppState,
    id: Uuid,
    poll_token: &str,
) -> Result<(Value, Option<String>), AppError> {
    let row = sqlx::query(
        r#"
        SELECT status, poll_token_hash, granted_token_ciphertext, user_agent, friendly_name, ip, created_at
        FROM login_approval
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("approval_not_found"))?;
    if row.get::<String, _>("poll_token_hash") != sha256_hex(poll_token) {
        return Err(AppError::not_found("approval_not_found"));
    }
    let status: String = row.get("status");
    if status != "allowed" {
        return Ok((
            json!({
                "status": status,
                "user_agent": row.get::<String, _>("user_agent"),
                "friendly_name": row.get::<String, _>("friendly_name"),
                "ip": row.get::<String, _>("ip"),
                "created_at": row.get::<chrono::DateTime<Utc>, _>("created_at"),
            }),
            None,
        ));
    }
    let packed: Option<Vec<u8>> = row.get("granted_token_ciphertext");
    let token = if let Some(packed) = packed {
        let raw = decrypt(&state.config.local_key, &packed)?;
        let token = String::from_utf8(raw).map_err(|_| AppError::internal())?;
        sqlx::query("UPDATE login_approval SET granted_token_ciphertext = NULL WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        Some(token)
    } else {
        None
    };
    Ok((
        json!({
            "status": "allowed",
            "user_agent": row.get::<String, _>("user_agent"),
            "friendly_name": row.get::<String, _>("friendly_name"),
            "ip": row.get::<String, _>("ip"),
        }),
        token,
    ))
}

pub fn session_payload(issued: &IssuedSession, username: &str, account_id: Uuid) -> Value {
    let mut body = login_body(account_id, username);
    if let Some(object) = body.as_object_mut() {
        object.insert("session_id".into(), json!(issued.session_id));
    }
    body
}

/// 登出：只吊销当前会话。
/// 自助改密保留当前会话并吊销其它会话；总管找回走 `revoke_all_sessions`，吊销全部会话。
pub async fn logout(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    sqlx::query("UPDATE session SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL")
        .bind(state.clock.now())
        .bind(user.session_id)
        .execute(&state.pool)
        .await?;
    Ok(json!({"revoked": true}))
}

/// 总管找回后吊销该账号全部会话（含当前），并作废未消费的邮箱 OTP。
pub async fn revoke_all_sessions(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query("UPDATE session SET revoked_at = $1 WHERE account_id = $2 AND revoked_at IS NULL")
        .bind(now)
        .bind(account_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "UPDATE magic_link SET consumed_at = $1 WHERE account_id = $2 AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(account_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM email_otp_challenge WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}
