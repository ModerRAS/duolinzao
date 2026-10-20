use std::io::{Cursor, Write};

use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::{audit, audit_user, email_enabled};
use crate::crypto::{hash_password, random_token, sha256_hex};
use crate::error::AppError;
use crate::extract::CurrentUser;
use crate::rbac::{authorize, Perm};
use crate::state::AppState;

pub async fn create_canteen(
    state: &AppState,
    user: &CurrentUser,
    name: &str,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::SystemSuper, None)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(AppError::bad("name_invalid", "食堂名称无效"));
    }
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO canteen (name, created_at) VALUES ($1, $2) RETURNING id")
            .bind(&name)
            .bind(state.clock.now())
            .fetch_one(&state.pool)
            .await?;
    audit_user(
        state,
        user,
        &hit,
        "canteen_create",
        Some("canteen"),
        Some(id.to_string()),
        json!({"name": name}),
    )
    .await?;
    Ok(json!({"id": id, "name": name}))
}

pub async fn list_canteens(state: &AppState, _user: &CurrentUser) -> Result<Value, AppError> {
    let rows = sqlx::query("SELECT id, name FROM canteen ORDER BY name")
        .fetch_all(&state.pool)
        .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "name": row.get::<String, _>("name"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn assign_role(
    state: &AppState,
    user: &CurrentUser,
    account_id: Uuid,
    role: &str,
    canteen_ids: Vec<Uuid>,
    source_label: Option<String>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::SystemSuper, None)?;
    if !matches!(role, "employee" | "canteen" | "admin" | "super_admin") {
        return Err(AppError::bad("role_invalid", "角色无效"));
    }
    if role == "super_admin" || role == "employee" {
        if !canteen_ids.is_empty() {
            return Err(AppError::bad("scope_invalid", "这个角色不带食堂范围"));
        }
    } else if canteen_ids.is_empty() {
        return Err(AppError::bad(
            "scope_invalid",
            "食堂和管理员分配必须带食堂范围",
        ));
    }
    for id in &canteen_ids {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM canteen WHERE id = $1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
        if !exists {
            return Err(AppError::bad("canteen_not_found", "食堂不存在"));
        }
    }
    let account_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM account WHERE id = $1)")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await?;
    if !account_exists {
        return Err(AppError::not_found("account_not_found"));
    }
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM role_assignment WHERE account_id = $1 AND role = $2")
        .bind(account_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        INSERT INTO role_assignment (account_id, role, canteen_ids, source_label, created_at)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(account_id)
    .bind(role)
    .bind(&canteen_ids)
    .bind(&source_label)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if role == "canteen" {
        sqlx::query("DELETE FROM canteen_staff WHERE account_id = $1")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        for id in &canteen_ids {
            sqlx::query("INSERT INTO canteen_staff (account_id, canteen_id) VALUES ($1, $2)")
                .bind(account_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    audit_user(
        state,
        user,
        &hit,
        "role_assign",
        Some("role_assignment"),
        Some(account_id.to_string()),
        json!({"role": role, "source_label": source_label}),
    )
    .await?;
    Ok(json!({"account_id": account_id, "role": role, "canteen_ids": canteen_ids}))
}

pub async fn create_invite(
    state: &AppState,
    user: &CurrentUser,
    max_uses: Option<i32>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::UserInvite, None)?;
    let max_uses = max_uses.unwrap_or(1);
    if max_uses < 1 {
        return Err(AppError::bad("max_uses_invalid", "可使用次数至少为 1"));
    }
    let code = random_token();
    let now = state.clock.now();
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO invite_code (code_hash, created_by, created_at, max_uses)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(sha256_hex(&code))
    .bind(user.account_id)
    .bind(now)
    .bind(max_uses)
    .fetch_one(&state.pool)
    .await?;
    audit_user(
        state,
        user,
        &hit,
        "invite_create",
        Some("invite"),
        Some(id.to_string()),
        json!({"max_uses": max_uses}),
    )
    .await?;
    Ok(json!({"id": id, "code": code, "created_at": now, "max_uses": max_uses}))
}

pub async fn update_invite_max_uses(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
    max_uses: i32,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::UserInvite, None)?;
    if max_uses < 1 {
        return Err(AppError::bad("max_uses_invalid", "可使用次数至少为 1"));
    }
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query("SELECT used_count FROM invite_code WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("invite_not_found"))?;
    let used_count: i32 = row.get("used_count");
    // 改小到等于 used_count 即立即用尽；小于 used_count 不允许。
    if max_uses < used_count {
        return Err(AppError::bad(
            "max_uses_invalid",
            "可使用次数不能小于已使用次数",
        ));
    }
    sqlx::query("UPDATE invite_code SET max_uses = $1 WHERE id = $2")
        .bind(max_uses)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::audit::write_audit(
        &mut *tx,
        crate::audit::AuditEntry {
            at: state.clock.now(),
            actor_id: Some(user.account_id),
            actor_username: Some(user.username.clone()),
            role: Some(hit.role.clone()),
            scope: hit.scope.clone(),
            action: "invite_update".into(),
            object_type: Some("invite".into()),
            object_id: Some(id.to_string()),
            summary: json!({"max_uses": max_uses}),
            result: "ok".into(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"id": id, "max_uses": max_uses, "used_count": used_count}))
}

pub async fn revoke_invite(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::UserInvite, None)?;
    let now = state.clock.now();
    let updated =
        sqlx::query("UPDATE invite_code SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL")
            .bind(now)
            .bind(id)
            .execute(&state.pool)
            .await?;
    if updated.rows_affected() == 0 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM invite_code WHERE id = $1)")
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
        if !exists {
            return Err(AppError::not_found("invite_not_found"));
        }
    }
    audit_user(
        state,
        user,
        &hit,
        "invite_revoke",
        Some("invite"),
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(json!({"id": id, "revoked": true}))
}

pub async fn list_invites(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::UserInvite, None)?;
    let rows = sqlx::query(
        r#"
        SELECT i.id, i.created_at, i.used_at, i.max_uses, i.used_count, i.revoked_at,
               a.username AS used_by, a.display_name AS used_by_display_name
        FROM invite_code i
        LEFT JOIN account a ON a.id = i.used_by
        ORDER BY i.created_at DESC
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            "used_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("used_at"),
            "used_by": row.get::<Option<String>, _>("used_by"),
            "used_by_display_name": row.get::<Option<String>, _>("used_by_display_name"),
            "max_uses": row.get::<i32, _>("max_uses"),
            "used_count": row.get::<i32, _>("used_count"),
            "revoked_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn set_email_enabled(
    state: &AppState,
    user: &CurrentUser,
    enabled: bool,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::SystemSuper, None)?;
    if enabled && !smtp_configured(state) {
        return Err(AppError::bad(
            "smtp_not_configured",
            "开启邮箱能力前需要在配置文件里填写 SMTP",
        ));
    }
    super::put_setting(
        &state.pool,
        "email_enabled",
        json!(enabled),
        state.clock.now(),
    )
    .await?;
    audit_user(
        state,
        user,
        &hit,
        "email_setting",
        Some("app_setting"),
        Some("email_enabled".into()),
        json!({"enabled": enabled}),
    )
    .await?;
    Ok(json!({"enabled": enabled}))
}

pub async fn get_email_enabled(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::SystemSuper, None)?;
    Ok(json!({
        "enabled": email_enabled(state).await?,
        "smtp_configured": smtp_configured(state),
    }))
}

fn smtp_configured(state: &AppState) -> bool {
    !state.config.smtp_host.is_empty()
        && !state.config.smtp_from.is_empty()
        && state.config.smtp_port != 0
}

pub async fn set_score_alert(
    state: &AppState,
    user: &CurrentUser,
    mean_threshold: f64,
    min_samples: i64,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::RatingStrongAlertConfig, None)?;
    if !mean_threshold.is_finite() || min_samples < 0 {
        return Err(AppError::bad("alert_config_invalid", "阈值或样本量无效"));
    }
    let now = state.clock.now();
    super::put_setting(&state.pool, "mean_threshold", json!(mean_threshold), now).await?;
    super::put_setting(&state.pool, "min_samples", json!(min_samples), now).await?;
    audit_user(
        state,
        user,
        &hit,
        "score_alert_config",
        Some("app_setting"),
        Some("score_alert".into()),
        json!({"mean_threshold": mean_threshold, "min_samples": min_samples}),
    )
    .await?;
    Ok(json!({"mean_threshold": mean_threshold, "min_samples": min_samples}))
}

pub async fn get_score_alert(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::RatingStrongAlertConfig, None)?;
    let (mean_threshold, min_samples) = super::alert_config(state).await?;
    Ok(json!({"mean_threshold": mean_threshold, "min_samples": min_samples}))
}

pub struct ChannelInput {
    pub id: Option<Uuid>,
    pub kind: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub enabled: bool,
}

pub async fn put_channels(
    state: &AppState,
    user: &CurrentUser,
    channels: Vec<ChannelInput>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::SystemSuper, None)?;
    let existing = sqlx::query("SELECT id, key_ciphertext FROM llm_channel")
        .fetch_all(&state.pool)
        .await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM llm_channel")
        .execute(&mut *tx)
        .await?;
    for (index, channel) in channels.iter().enumerate() {
        if !matches!(
            channel.kind.as_str(),
            "local_script" | "openai_chat" | "openai_responses" | "anthropic"
        ) {
            return Err(AppError::bad("channel_kind_invalid", "渠道种类无效"));
        }
        // 出站收紧：只允许 https（http 仅回环测试例外），禁字面私网/链路本地。
        if channel.kind != "local_script" {
            if let Some(url) = channel.base_url.as_deref() {
                crate::domain::validate_llm_base_url(url).map_err(|_| {
                    AppError::bad(
                        "channel_base_url_invalid",
                        "base_url 只允许 https（http 仅限回环测试），且不能指向私网/链路本地",
                    )
                })?;
            }
        }
        let previous = channel.id.and_then(|id| {
            existing
                .iter()
                .find(|row| row.get::<Uuid, _>("id") == id)
                .map(|row| row.get::<Option<Vec<u8>>, _>("key_ciphertext"))
        });
        let ciphertext = if channel.kind == "local_script" {
            None
        } else if let Some(key) = channel.api_key.as_ref().filter(|key| !key.is_empty()) {
            Some(crate::crypto::encrypt(
                &state.config.local_key,
                key.as_bytes(),
            )?)
        } else if let Some(Some(old)) = previous {
            Some(old)
        } else if let Some(None) = previous {
            return Err(AppError::bad("channel_key_required", "渠道需要 key"));
        } else {
            return Err(AppError::bad("channel_key_required", "渠道需要 key"));
        };
        sqlx::query(
            r#"
            INSERT INTO llm_channel (position, kind, base_url, model, key_ciphertext, enabled, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(index as i32)
        .bind(&channel.kind)
        .bind(&channel.base_url)
        .bind(&channel.model)
        .bind(&ciphertext)
        .bind(channel.enabled)
        .bind(state.clock.now())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    audit_user(
        state,
        user,
        &hit,
        "llm_channels",
        Some("llm_channel"),
        None,
        json!({
            // base_url 变更必须留痕：记 kind + base_url + model，不记 key 明文。
            "channels": channels.iter().enumerate().map(|(index, channel)| json!({
                "position": index,
                "kind": channel.kind,
                "base_url": channel.base_url,
                "model": channel.model,
                "enabled": channel.enabled,
                "key_configured": channel.kind == "local_script" || channel.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(channel.id.is_some()),
            })).collect::<Vec<_>>()
        }),
    )
    .await?;
    list_channels(state, user).await
}

pub async fn list_channels(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::SystemSuper, None)?;
    let rows = sqlx::query(
        r#"
        SELECT id, position, kind, base_url, model, key_ciphertext IS NOT NULL AS key_configured, enabled
        FROM llm_channel
        ORDER BY position
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "channels": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "position": row.get::<i32, _>("position"),
            "kind": row.get::<String, _>("kind"),
            "base_url": row.get::<Option<String>, _>("base_url"),
            "model": row.get::<Option<String>, _>("model"),
            "enabled": row.get::<bool, _>("enabled"),
            "key_configured": row.get::<bool, _>("key_configured"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn recover(
    state: &AppState,
    user: &CurrentUser,
    username: &str,
    action: &str,
    password: Option<String>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::UserRecover, None)?;
    let account = sqlx::query("SELECT id FROM account WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("account_not_found"))?;
    let account_id: Uuid = account.get("id");
    let now = state.clock.now();
    match action {
        "reset_password" => {
            let password = password.unwrap_or_default();
            super::ensure_password(&password)?;
            let hash = hash_password(&password)?;
            sqlx::query("UPDATE account SET password_hash = $1 WHERE id = $2")
                .bind(hash)
                .bind(account_id)
                .execute(&state.pool)
                .await?;
            // 改密后旧会话必须立即失效，否则盗用的 cookie 还能继续用。
            super::session::revoke_all_sessions(&state.pool, account_id, now).await?;
        }
        "clear_credentials" => {
            sqlx::query(
                "UPDATE passkey SET revoked_at = $1 WHERE account_id = $2 AND revoked_at IS NULL",
            )
            .bind(now)
            .bind(account_id)
            .execute(&state.pool)
            .await?;
            sqlx::query("DELETE FROM totp_credential WHERE account_id = $1")
                .bind(account_id)
                .execute(&state.pool)
                .await?;
            sqlx::query(
                "UPDATE session SET revoked_at = $1 WHERE account_id = $2 AND revoked_at IS NULL",
            )
            .bind(now)
            .bind(account_id)
            .execute(&state.pool)
            .await?;
        }
        _ => return Err(AppError::bad("recovery_invalid", "找回动作无效")),
    }
    audit_user(
        state,
        user,
        &hit,
        "account_recover",
        Some("account"),
        Some(account_id.to_string()),
        json!({"action": action, "username": username}),
    )
    .await?;
    Ok(json!({"username": username, "action": action, "done": true}))
}

pub async fn list_accounts(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    if authorize(&user.assignments, Perm::UserRecover, None).is_err()
        && authorize(&user.assignments, Perm::SystemSuper, None).is_err()
    {
        return Err(AppError::forbidden());
    }
    let rows = sqlx::query(
        r#"
        SELECT a.id, a.username, a.display_name, a.email,
               a.password_hash IS NOT NULL AS has_password,
               (SELECT COUNT(*) FROM passkey p WHERE p.account_id = a.id AND p.revoked_at IS NULL) AS passkey_count,
               EXISTS(SELECT 1 FROM totp_credential t WHERE t.account_id = a.id AND t.confirmed) AS totp
        FROM account a
        ORDER BY a.username
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "username": row.get::<String, _>("username"),
            "display_name": row.get::<Option<String>, _>("display_name"),
            "email": row.get::<Option<String>, _>("email"),
            "has_password": row.get::<bool, _>("has_password"),
            "passkey_count": row.get::<i64, _>("passkey_count"),
            "totp": row.get::<bool, _>("totp"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn list_audit(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::AuditRead, None)?;
    let rows = sqlx::query(
        r#"
        SELECT id, created_at, actor_id, actor_username, role, scope, action, object_type, object_id, summary, result
        FROM audit_log
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            "actor_id": row.get::<Option<Uuid>, _>("actor_id"),
            "actor_username": row.get::<Option<String>, _>("actor_username"),
            "role": row.get::<Option<String>, _>("role"),
            "scope": row.get::<String, _>("scope"),
            "action": row.get::<String, _>("action"),
            "object_type": row.get::<Option<String>, _>("object_type"),
            "object_id": row.get::<Option<String>, _>("object_id"),
            "summary": row.get::<Value, _>("summary"),
            "result": row.get::<String, _>("result"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn export_zip(state: &AppState, user: &CurrentUser) -> Result<Vec<u8>, AppError> {
    let hit = authorize(&user.assignments, Perm::ExportRun, None)?;
    let canteens = sqlx::query("SELECT id, name, created_at FROM canteen ORDER BY name")
        .fetch_all(&state.pool)
        .await?;
    let dishes = sqlx::query(
        "SELECT id, canteen_id, name, kind, taste_tags, image_path, enabled FROM dish ORDER BY name",
    )
    .fetch_all(&state.pool)
    .await?;
    let orders = sqlx::query(
        r#"
        SELECT id, account_id, meal_date, slot, canteen_id, ordering, source, submitted_at, locked_at
        FROM meal_order ORDER BY meal_date, slot
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let scores = sqlx::query(
        "SELECT id, account_id, meal_date, slot, dish_id, value, created_at FROM score ORDER BY created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    let comments = sqlx::query(
        r#"
        SELECT c.id, c.body, c.anonymous, c.deleted_at, s.meal_date, s.slot, s.dish_id,
               CASE WHEN c.anonymous THEN NULL ELSE s.account_id END AS account_id
        FROM comment c
        JOIN score s ON s.id = c.score_id
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let audits = sqlx::query(
        r#"
        SELECT id, created_at, actor_username, role, scope, action, object_type, object_id, summary, result
        FROM audit_log ORDER BY created_at
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut write_json = |name: &str, value: Value| -> Result<(), AppError> {
        zip.start_file(name, opts)
            .map_err(|_| AppError::internal())?;
        let bytes = serde_json::to_vec_pretty(&value).map_err(|_| AppError::internal())?;
        zip.write_all(&bytes).map_err(|_| AppError::internal())?;
        Ok(())
    };
    write_json(
        "canteens.json",
        json!(canteens
            .iter()
            .map(|row| json!({
                "id": row.get::<Uuid, _>("id"),
                "name": row.get::<String, _>("name"),
            }))
            .collect::<Vec<_>>()),
    )?;
    write_json(
        "dishes.json",
        json!(dishes
            .iter()
            .map(|row| json!({
                "id": row.get::<Uuid, _>("id"),
                "canteen_id": row.get::<Uuid, _>("canteen_id"),
                "name": row.get::<String, _>("name"),
                "kind": row.get::<String, _>("kind"),
                "taste_tags": row.get::<Vec<String>, _>("taste_tags"),
                "image_path": row.get::<Option<String>, _>("image_path"),
                "enabled": row.get::<bool, _>("enabled"),
            }))
            .collect::<Vec<_>>()),
    )?;
    write_json(
        "meal_orders.json",
        json!(orders
            .iter()
            .map(|row| json!({
                "id": row.get::<Uuid, _>("id"),
                "account_id": row.get::<Uuid, _>("account_id"),
                "meal_date": row.get::<chrono::NaiveDate, _>("meal_date"),
                "slot": row.get::<String, _>("slot"),
                "canteen_id": row.get::<Option<Uuid>, _>("canteen_id"),
                "ordering": row.get::<bool, _>("ordering"),
                "source": row.get::<String, _>("source"),
            }))
            .collect::<Vec<_>>()),
    )?;
    write_json(
        "scores.json",
        json!(scores
            .iter()
            .map(|row| json!({
                "id": row.get::<Uuid, _>("id"),
                "account_id": row.get::<Uuid, _>("account_id"),
                "meal_date": row.get::<chrono::NaiveDate, _>("meal_date"),
                "slot": row.get::<String, _>("slot"),
                "dish_id": row.get::<Uuid, _>("dish_id"),
                "value": row.get::<i16, _>("value"),
            }))
            .collect::<Vec<_>>()),
    )?;
    write_json(
        "comments.json",
        json!(comments
            .iter()
            .map(|row| json!({
                "id": row.get::<Uuid, _>("id"),
                "body": row.get::<String, _>("body"),
                "anonymous": row.get::<bool, _>("anonymous"),
                "account_id": row.get::<Option<Uuid>, _>("account_id"),
                "meal_date": row.get::<chrono::NaiveDate, _>("meal_date"),
                "slot": row.get::<String, _>("slot"),
                "dish_id": row.get::<Uuid, _>("dish_id"),
            }))
            .collect::<Vec<_>>()),
    )?;
    write_json(
        "audit.json",
        json!(audits
            .iter()
            .map(|row| json!({
                "id": row.get::<Uuid, _>("id"),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "actor_username": row.get::<Option<String>, _>("actor_username"),
                "role": row.get::<Option<String>, _>("role"),
                "scope": row.get::<String, _>("scope"),
                "action": row.get::<String, _>("action"),
                "object_type": row.get::<Option<String>, _>("object_type"),
                "object_id": row.get::<Option<String>, _>("object_id"),
                "summary": row.get::<Value, _>("summary"),
                "result": row.get::<String, _>("result"),
            }))
            .collect::<Vec<_>>()),
    )?;
    for row in &dishes {
        if let Some(path) = row.get::<Option<String>, _>("image_path") {
            if path.contains('/') || path.contains("..") {
                continue;
            }
            let file = std::path::Path::new(&state.config.dish_image_dir).join(&path);
            if let Ok(bytes) = std::fs::read(file) {
                zip.start_file(format!("images/{path}"), opts)
                    .map_err(|_| AppError::internal())?;
                zip.write_all(&bytes).map_err(|_| AppError::internal())?;
            }
        }
    }
    let cursor = zip.finish().map_err(|_| AppError::internal())?;
    audit(
        state,
        Some(user.account_id),
        Some(user.username.clone()),
        Some(&hit),
        "export",
        Some("export"),
        None,
        json!({"files": ["canteens", "dishes", "meal_orders", "scores", "comments", "audit", "images"]}),
        "ok",
    )
    .await?;
    Ok(cursor.into_inner())
}
