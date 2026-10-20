use chrono::{Duration, Utc};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;
use webauthn_rs::prelude::{Passkey, PublicKeyCredential, RegisterPublicKeyCredential};

use super::{
    audit, audit_user, check_login_rate, email_enabled, ensure_password, ensure_username,
    issue_session, login_body, normalize_display_name, record_login, too_many_failures,
    IssuedSession, MAGIC_LINK_MINUTES, PASSKEY_CHALLENGE_MINUTES,
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
        let password_hash = hash_password(&state.config.super_password)?;
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
        methods.push("magic_link");
    }
    methods.push("password");
    Ok(json!({
        "methods": methods,
        "email_enabled": enabled,
    }))
}

pub async fn register(
    state: &AppState,
    username: &str,
    invite_code: &str,
    password: &str,
) -> Result<Value, AppError> {
    let username = ensure_username(username)?;
    ensure_password(password)?;
    if invite_code.trim().is_empty() {
        return Err(AppError::bad("invite_invalid", "邀请码无效"));
    }
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    let invite = sqlx::query(
        r#"
        SELECT id, max_uses, used_count, revoked_at FROM invite_code
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
    let password_hash = hash_password(password)?;
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
    sqlx::query(
        r#"
        INSERT INTO role_assignment (account_id, role, canteen_ids, created_at)
        VALUES ($1, 'employee', '{}', $2)
        "#,
    )
    .bind(account_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
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
            role: Some("employee".into()),
            scope: "self".into(),
            action: "register".into(),
            object_type: Some("account".into()),
            object_id: Some(account_id.to_string()),
            summary: json!({"username": username}),
            result: "ok".into(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"account_id": account_id, "username": username}))
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
    let ok = row
        .as_ref()
        .and_then(|item| item.get::<Option<String>, _>("password_hash"))
        .map(|hash| verify_password(password, &hash))
        .unwrap_or(false);
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

/// 自助改密：校验当前密码，保留当前会话，吊销该账号其它会话，并作废未消费的 magic link。
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
    if !verify_password(current_password, &hash) {
        drop(tx);
        record_login(&state.pool, &user.username, ip, false, now).await?;
        return Err(AppError::bad("password_incorrect", "当前密码不正确"));
    }
    if current_password == new_password {
        return Err(AppError::bad("password_unchanged", "新密码与当前密码相同"));
    }
    ensure_password(new_password)?;
    let new_hash = hash_password(new_password)?;
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
    if !check_totp(state, user.account_id, &user.username, code).await? {
        return Err(AppError::bad("totp_invalid", "验证码不正确"));
    }
    sqlx::query("UPDATE totp_credential SET confirmed = TRUE WHERE account_id = $1")
        .bind(user.account_id)
        .execute(&state.pool)
        .await?;
    Ok(json!({"confirmed": true}))
}

async fn check_totp(
    state: &AppState,
    account_id: Uuid,
    username: &str,
    code: &str,
) -> Result<bool, AppError> {
    let row = sqlx::query("SELECT secret_ciphertext FROM totp_credential WHERE account_id = $1")
        .bind(account_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Ok(false);
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
    Ok(totp.check(code.trim(), now))
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
    if !check_totp(state, account_id, &username, code).await? {
        record_login(&state.pool, &username, ip, false, now).await?;
        return Err(AppError::bad("totp_invalid", "验证码不正确"));
    }
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

pub async fn request_magic_link(state: &AppState, username: &str) -> Result<Value, AppError> {
    if !email_enabled(state).await? {
        return Err(AppError::bad("email_disabled", "邮箱能力已关闭"));
    }
    let row = sqlx::query("SELECT id, email FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Ok(json!({"status": "accepted"}));
    };
    let Some(email) = row.get::<Option<String>, _>("email") else {
        return Err(AppError::bad(
            "email_not_bound",
            "使用 magic link 前需要先绑定邮箱",
        ));
    };
    let now = state.clock.now();
    let token = random_token();
    sqlx::query(
        r#"
        INSERT INTO magic_link (account_id, token_hash, expires_at, created_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(row.get::<Uuid, _>("id"))
    .bind(sha256_hex(&token))
    .bind(now + Duration::minutes(MAGIC_LINK_MINUTES))
    .bind(now)
    .execute(&state.pool)
    .await?;
    let link = format!("{}/login?magic={token}", state.config.webauthn_origin);
    let body =
        format!("多邻灶登录链接：{link}\ntoken={token}\n此链接 15 分钟内有效，且只能使用一次。");
    state
        .mailer
        .send(&email, "多邻灶登录", &body)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "magic link mail failed");
            AppError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "mail_failed",
                "邮件发送失败",
            )
        })?;
    Ok(json!({"status": "accepted"}))
}

pub async fn consume_magic_link(
    state: &AppState,
    token: &str,
    user_agent: &str,
    ip: &str,
) -> Result<IssuedSession, AppError> {
    if !email_enabled(state).await? {
        return Err(AppError::bad("email_disabled", "邮箱能力已关闭"));
    }
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        r#"
        SELECT m.id, m.account_id, a.username
        FROM magic_link m
        JOIN account a ON a.id = m.account_id
        WHERE m.token_hash = $1 AND m.consumed_at IS NULL AND m.expires_at > $2
        FOR UPDATE
        "#,
    )
    .bind(sha256_hex(token))
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::bad("magic_invalid", "链接无效或已使用"))?;
    sqlx::query("UPDATE magic_link SET consumed_at = $1 WHERE id = $2")
        .bind(now)
        .bind(row.get::<Uuid, _>("id"))
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE account SET email_verified = TRUE WHERE id = $1")
        .bind(row.get::<Uuid, _>("account_id"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let account_id: Uuid = row.get("account_id");
    let username: String = row.get("username");
    let issued = issue_session(&state.pool, now, account_id, user_agent, ip).await?;
    audit(
        state,
        Some(account_id),
        Some(username),
        None,
        "login",
        Some("session"),
        Some(issued.session_id.to_string()),
        json!({"method": "magic_link"}),
        "ok",
    )
    .await?;
    Ok(issued)
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

pub async fn passkey_auth_start(state: &AppState, username: &str) -> Result<Value, AppError> {
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

pub async fn passkey_auth_finish(
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
    let mut rows = sqlx::query(
        "SELECT id, passkey_json FROM passkey WHERE account_id = $1 AND revoked_at IS NULL",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await?;
    let mut matched = false;
    for row in &mut rows {
        let mut passkey: Passkey =
            serde_json::from_value(row.get("passkey_json")).map_err(|_| AppError::internal())?;
        if passkey.cred_id().as_ref() == result.cred_id().as_ref() {
            matched = true;
            if passkey.update_credential(&result) == Some(true) {
                let doc = serde_json::to_value(&passkey).map_err(|_| AppError::internal())?;
                sqlx::query("UPDATE passkey SET passkey_json = $1 WHERE id = $2")
                    .bind(doc)
                    .bind(row.get::<Uuid, _>("id"))
                    .execute(&state.pool)
                    .await?;
            }
            break;
        }
    }
    if !matched {
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

/// 总管找回后吊销该账号全部会话（含当前），并作废未消费的 magic link。
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
    Ok(())
}
