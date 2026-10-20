use chrono::{Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use super::{audit_user, GENERATE_HORIZON_DAYS};
use crate::domain::{
    dsl_prompt, local_script_dsl, meal_is_locked, order_color, parse_dsl_text, resolve_meal,
    shanghai_at, shanghai_date, upsert_override, valid_slot, MealDsl, SLOTS,
};
use crate::error::AppError;
use crate::extract::CurrentUser;
use crate::rbac::{authorize, Perm};
use crate::state::AppState;

pub async fn put_shift(
    state: &AppState,
    user: &CurrentUser,
    kind: &str,
    anchor_date: Option<NaiveDate>,
    canteen_id: Option<Uuid>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::ShiftSelfWrite, None)?;
    if kind != "work2_rest4" && kind != "custom" {
        return Err(AppError::bad(
            "shift_kind_invalid",
            "班制只有上二休四和自定义",
        ));
    }
    if kind == "work2_rest4" && anchor_date.is_none() {
        return Err(AppError::bad("anchor_required", "上二休四需要锚点日"));
    }
    let Some(canteen_id) = canteen_id else {
        return Err(AppError::bad("canteen_required", "请先绑定食堂"));
    };
    ensure_member(state, user.account_id, canteen_id).await?;
    let now = state.clock.now();
    let dsl = if kind == "work2_rest4" {
        MealDsl::work2(anchor_date.unwrap(), Some(canteen_id))
    } else {
        MealDsl::custom_blank(Some(canteen_id))
    };
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO shift_plan (account_id, kind, anchor_date, updated_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (account_id) DO UPDATE
        SET kind = EXCLUDED.kind, anchor_date = EXCLUDED.anchor_date, updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(user.account_id)
    .bind(kind)
    .bind(anchor_date)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let template_id = upsert_template(&mut tx, user.account_id, &dsl, now).await?;
    tx.commit().await?;
    let written = generate_range(state, user.account_id, horizon(state)).await?;
    audit_user(
        state,
        user,
        &hit,
        "shift_set",
        Some("shift"),
        Some(user.account_id.to_string()),
        json!({"kind": kind}),
    )
    .await?;
    Ok(json!({
        "id": template_id,
        "dsl": dsl,
        "generated": written,
    }))
}

pub async fn get_shift(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::ShiftSelfWrite, None)
        .or_else(|_| authorize(&user.assignments, Perm::MealSelfRead, None))?;
    let row = sqlx::query("SELECT kind, anchor_date FROM shift_plan WHERE account_id = $1")
        .bind(user.account_id)
        .fetch_optional(&state.pool)
        .await?;
    let template = load_template(state, user.account_id).await?;
    Ok(json!({
        "kind": row.as_ref().map(|item| item.get::<String, _>("kind")),
        "anchor_date": row.as_ref().map(|item| item.get::<Option<NaiveDate>, _>("anchor_date")),
        "template_id": template.as_ref().map(|item| item.0),
        "dsl": template.as_ref().map(|item| &item.1),
    }))
}

pub async fn get_template(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::MealSelfRead, None)?;
    let Some((id, dsl)) = load_template(state, user.account_id).await? else {
        return Ok(json!({"id": null, "dsl": null, "preview": []}));
    };
    let now = state.clock.now();
    Ok(json!({
        "id": id,
        "dsl": dsl,
        "preview": preview(&dsl, shanghai_date(now), 7, state.config.lock_time, now),
    }))
}

pub async fn put_template(
    state: &AppState,
    user: &CurrentUser,
    template_id: Uuid,
    mut dsl: MealDsl,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MealSelfWrite, None)?;
    if !dsl.valid_mode() {
        return Err(AppError::bad(
            "shift_kind_invalid",
            "班制只有上二休四和自定义",
        ));
    }
    ensure_template_owner(state, user.account_id, template_id).await?;
    dsl.version = 1;
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE meal_template SET dsl = $1, updated_at = $2 WHERE id = $3")
        .bind(serde_json::to_value(&dsl).map_err(|_| AppError::internal())?)
        .bind(now)
        .bind(template_id)
        .execute(&mut *tx)
        .await?;
    sync_shift(&mut tx, user.account_id, &dsl, now).await?;
    tx.commit().await?;
    let written = generate_range(state, user.account_id, horizon(state)).await?;
    audit_user(
        state,
        user,
        &hit,
        "meal_template_update",
        Some("meal_template"),
        Some(template_id.to_string()),
        json!({"mode": dsl.mode}),
    )
    .await?;
    Ok(json!({"id": template_id, "dsl": dsl, "generated": written}))
}

pub async fn put_slot(
    state: &AppState,
    user: &CurrentUser,
    template_id: Uuid,
    date: NaiveDate,
    slot: &str,
    ordering: bool,
    canteen_id: Option<Uuid>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MealSelfWrite, None)?;
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    ensure_template_owner(state, user.account_id, template_id).await?;
    if ordering {
        let canteen_id =
            canteen_id.ok_or_else(|| AppError::bad("canteen_required", "报餐需要选择食堂"))?;
        ensure_member(state, user.account_id, canteen_id).await?;
    }
    let mut dsl = load_template_by_id(state, template_id).await?;
    upsert_override(&mut dsl, date, slot, ordering, canteen_id);
    let now = state.clock.now();
    sqlx::query("UPDATE meal_template SET dsl = $1, updated_at = $2 WHERE id = $3")
        .bind(serde_json::to_value(&dsl).map_err(|_| AppError::internal())?)
        .bind(now)
        .bind(template_id)
        .execute(&state.pool)
        .await?;
    let locked = meal_is_locked(date, state.config.lock_time, now);
    let mut order_updated = false;
    if !locked {
        persist_lock_if_needed(state, date).await?;
        order_updated = write_resolved(state, user.account_id, date, slot, &dsl, "dsl").await?;
    }
    audit_user(
        state,
        user,
        &hit,
        "meal_dsl_edit",
        Some("meal_template"),
        Some(template_id.to_string()),
        json!({"date": date, "slot": slot, "ordering": ordering, "locked": locked}),
    )
    .await?;
    Ok(json!({
        "dsl": dsl,
        "dsl_updated": true,
        "order_updated": order_updated,
        "locked": locked,
        "color": order_color(ordering),
    }))
}

pub async fn apply_template(
    state: &AppState,
    user: &CurrentUser,
    template_id: Uuid,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MealSelfWrite, None)?;
    if end < start || (end - start).num_days() > 62 {
        return Err(AppError::bad("range_invalid", "套用日期范围无效"));
    }
    ensure_template_owner(state, user.account_id, template_id).await?;
    let written = generate_range(state, user.account_id, start..=end).await?;
    audit_user(
        state,
        user,
        &hit,
        "meal_template_apply",
        Some("meal_template"),
        Some(template_id.to_string()),
        json!({"start": start, "end": end, "written": written}),
    )
    .await?;
    Ok(json!({"written": written}))
}

pub async fn put_meal(
    state: &AppState,
    user: &CurrentUser,
    date: NaiveDate,
    slot: &str,
    ordering: bool,
    canteen_id: Option<Uuid>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MealSelfWrite, None)?;
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    let now = state.clock.now();
    if meal_is_locked(date, state.config.lock_time, now) {
        persist_lock_if_needed(state, date).await?;
        return Err(AppError::new(
            axum::http::StatusCode::FORBIDDEN,
            "meal_locked",
            "该餐已锁定，仅管理员或总管可在范围内改单",
        ));
    }
    if ordering {
        let canteen_id =
            canteen_id.ok_or_else(|| AppError::bad("canteen_required", "报餐需要选择食堂"))?;
        ensure_member(state, user.account_id, canteen_id).await?;
    }
    let mut dsl = match load_template(state, user.account_id).await? {
        Some((_, dsl)) => dsl,
        None => MealDsl::custom_blank(canteen_id),
    };
    upsert_override(&mut dsl, date, slot, ordering, canteen_id);
    let mut tx = state.pool.begin().await?;
    let template_id = upsert_template(&mut tx, user.account_id, &dsl, now).await?;
    tx.commit().await?;
    upsert_order(
        state,
        user.account_id,
        date,
        slot,
        canteen_id.or(dsl.canteen_id),
        ordering,
        "manual",
    )
    .await?;
    audit_user(
        state,
        user,
        &hit,
        "meal_write",
        Some("meal_order"),
        Some(format!("{date}/{slot}")),
        json!({"ordering": ordering, "template_id": template_id}),
    )
    .await?;
    Ok(json!({
        "date": date,
        "slot": slot,
        "ordering": ordering,
        "canteen_id": canteen_id,
        "color": order_color(ordering),
        "template_id": template_id,
    }))
}

pub async fn list_meals(
    state: &AppState,
    user: &CurrentUser,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::MealSelfRead, None)?;
    let rows = sqlx::query(
        r#"
        SELECT meal_date, slot, canteen_id, ordering, source, locked_at
        FROM meal_order
        WHERE account_id = $1 AND meal_date >= $2 AND meal_date <= $3
        ORDER BY meal_date, slot
        "#,
    )
    .bind(user.account_id)
    .bind(start)
    .bind(end)
    .fetch_all(&state.pool)
    .await?;
    let now = state.clock.now();
    let mut locked_dates = Vec::new();
    if end >= start {
        let mut cursor = start;
        loop {
            if meal_is_locked(cursor, state.config.lock_time, now) {
                locked_dates.push(cursor);
            }
            if cursor >= end {
                break;
            }
            cursor += Duration::days(1);
        }
    }
    Ok(json!({
        "items": rows.iter().map(|row| {
            let date: NaiveDate = row.get("meal_date");
            let ordering: bool = row.get("ordering");
            json!({
                "date": date,
                "slot": row.get::<String, _>("slot"),
                "canteen_id": row.get::<Option<Uuid>, _>("canteen_id"),
                "ordering": ordering,
                "source": row.get::<String, _>("source"),
                "locked": row.get::<Option<chrono::DateTime<Utc>>, _>("locked_at").is_some()
                    || meal_is_locked(date, state.config.lock_time, now),
                "color": order_color(ordering),
            })
        }).collect::<Vec<_>>(),
        "locked_dates": locked_dates,
    }))
}

pub async fn admin_put_meal(
    state: &AppState,
    user: &CurrentUser,
    account_id: Uuid,
    date: NaiveDate,
    slot: &str,
    ordering: bool,
    canteen_id: Option<Uuid>,
) -> Result<Value, AppError> {
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    let existing = sqlx::query(
        "SELECT canteen_id FROM meal_order WHERE account_id = $1 AND meal_date = $2 AND slot = $3",
    )
    .bind(account_id)
    .bind(date)
    .bind(slot)
    .fetch_optional(&state.pool)
    .await?;
    let mut needed = Vec::new();
    if let Some(id) = canteen_id {
        needed.push(id);
    }
    if let Some(row) = &existing {
        if let Some(id) = row.get::<Option<Uuid>, _>("canteen_id") {
            needed.push(id);
        }
    }
    if needed.is_empty() {
        return Err(AppError::bad("canteen_required", "改单需要能定位食堂"));
    }
    let mut hit = None;
    for canteen in needed {
        hit = Some(authorize(
            &user.assignments,
            Perm::MealAdminOverride,
            Some(canteen),
        )?);
    }
    let hit = hit.unwrap();
    let now = state.clock.now();
    if meal_is_locked(date, state.config.lock_time, now) {
        persist_lock_if_needed(state, date).await?;
    }
    // 管理员改单已在上文鉴权（meal.admin_override + 食堂范围），这里必须能复写已锁定的行。
    // upsert_order 的 WHERE locked_at IS NULL 是防员工/任务写锁单，不能挡管理员，所以这里直写。
    sqlx::query(
        r#"
        INSERT INTO meal_order (account_id, meal_date, slot, canteen_id, ordering, source, submitted_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (account_id, meal_date, slot) DO UPDATE
        SET canteen_id = EXCLUDED.canteen_id,
            ordering = EXCLUDED.ordering,
            source = EXCLUDED.source,
            submitted_at = EXCLUDED.submitted_at
        "#,
    )
    .bind(account_id)
    .bind(date)
    .bind(slot)
    .bind(canteen_id)
    .bind(ordering)
    .bind("admin")
    .bind(now)
    .execute(&state.pool)
    .await?;
    if meal_is_locked(date, state.config.lock_time, now) {
        sqlx::query(
            r#"
            UPDATE meal_order SET locked_at = $4
            WHERE account_id = $1 AND meal_date = $2 AND slot = $3 AND locked_at IS NULL
            "#,
        )
        .bind(account_id)
        .bind(date)
        .bind(slot)
        .bind(now)
        .execute(&state.pool)
        .await?;
    }
    audit_user(
        state,
        user,
        &hit,
        "meal_admin_override",
        Some("meal_order"),
        Some(format!("{account_id}/{date}/{slot}")),
        json!({"ordering": ordering, "canteen_id": canteen_id}),
    )
    .await?;
    Ok(json!({
        "account_id": account_id,
        "date": date,
        "slot": slot,
        "ordering": ordering,
        "canteen_id": canteen_id,
    }))
}

pub async fn admin_list_meals(
    state: &AppState,
    user: &CurrentUser,
    date: NaiveDate,
    canteen_id: Uuid,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MealAdminOverride, Some(canteen_id))?;
    let _ = hit;
    let rows = sqlx::query(
        r#"
        SELECT o.account_id, a.username, a.display_name, o.meal_date, o.slot, o.canteen_id, o.ordering, o.locked_at
        FROM meal_order o
        JOIN account a ON a.id = o.account_id
        WHERE o.meal_date = $1 AND o.canteen_id = $2
        ORDER BY a.username, o.slot
        "#,
    )
    .bind(date)
    .bind(canteen_id)
    .fetch_all(&state.pool)
    .await?;
    let locked = meal_is_locked(date, state.config.lock_time, state.clock.now());
    Ok(json!({
        "locked": locked,
        "items": rows.iter().map(|row| json!({
            "account_id": row.get::<Uuid, _>("account_id"),
            "username": row.get::<String, _>("username"),
            "display_name": row.get::<Option<String>, _>("display_name"),
            "date": row.get::<NaiveDate, _>("meal_date"),
            "slot": row.get::<String, _>("slot"),
            "canteen_id": row.get::<Option<Uuid>, _>("canteen_id"),
            "ordering": row.get::<bool, _>("ordering"),
            "locked_at": row.get::<Option<chrono::DateTime<Utc>>, _>("locked_at"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn generate_template(
    state: &AppState,
    user: &CurrentUser,
    description: &str,
    anchor_date: Option<NaiveDate>,
    canteen_id: Option<Uuid>,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::ShiftSelfWrite, None)?;
    if description.trim().is_empty() || description.chars().count() > 2000 {
        return Err(AppError::bad("description_invalid", "班制描述不能为空"));
    }
    if description.contains("上五休二") {
        return Err(AppError::bad("shift_kind_invalid", "一期没有上五休二"));
    }
    let channels = sqlx::query(
        r#"
        SELECT position, kind, base_url, model, key_ciphertext, enabled
        FROM llm_channel
        ORDER BY position, created_at
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let today = shanghai_date(state.clock.now());
    let prompt = dsl_prompt(description);
    let mut dsl = None::<MealDsl>;
    let mut parse_failed = false;
    for row in channels {
        if !row.get::<bool, _>("enabled") {
            continue;
        }
        let kind: String = row.get("kind");
        state.templates.record(&kind);
        if kind == "local_script" {
            match local_script_dsl(description, anchor_date, canteen_id, today) {
                Ok(parsed) => {
                    dsl = Some(parsed);
                    break;
                }
                Err(err) => {
                    tracing::info!(error = %err, "local script skipped");
                    if err == "dsl_parse" {
                        parse_failed = true;
                    }
                    continue;
                }
            }
        }
        let key = match row.get::<Option<Vec<u8>>, _>("key_ciphertext") {
            Some(packed) => String::from_utf8(
                crate::crypto::decrypt(&state.config.local_key, &packed).unwrap_or_default(),
            )
            .unwrap_or_default(),
            None => String::new(),
        };
        if key.is_empty() {
            continue;
        }
        let text = match state
            .templates
            .complete(
                &kind,
                row.get::<Option<String>, _>("base_url")
                    .as_deref()
                    .unwrap_or(""),
                row.get::<Option<String>, _>("model")
                    .as_deref()
                    .unwrap_or(""),
                &key,
                &prompt,
            )
            .await
        {
            Ok(text) => text,
            Err(err) => {
                tracing::info!(kind = %kind, error = %err, "channel failed");
                if err == "dsl_parse" {
                    parse_failed = true;
                }
                continue;
            }
        };
        match parse_dsl_text(&text) {
            Ok(parsed) => {
                dsl = Some(parsed);
                break;
            }
            Err(_) => parse_failed = true,
        }
    }
    let Some(mut dsl) = dsl else {
        if parse_failed {
            return Err(AppError::bad("dsl_parse", "解析失败"));
        }
        return Err(AppError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "template_unavailable",
            "没有可用的模板渠道",
        ));
    };
    if canteen_id.is_some() && dsl.canteen_id.is_none() {
        dsl.canteen_id = canteen_id;
    }
    if anchor_date.is_some() && dsl.anchor_date.is_none() && dsl.mode == "work2_rest4" {
        dsl.anchor_date = anchor_date;
    }
    let now = state.clock.now();
    let mut tx = state.pool.begin().await?;
    let template_id = upsert_template(&mut tx, user.account_id, &dsl, now).await?;
    sync_shift(&mut tx, user.account_id, &dsl, now).await?;
    tx.commit().await?;
    let written = generate_range(state, user.account_id, horizon(state)).await?;
    audit_user(
        state,
        user,
        &hit,
        "meal_template_generate",
        Some("meal_template"),
        Some(template_id.to_string()),
        json!({"mode": dsl.mode}),
    )
    .await?;
    Ok(json!({
        "id": template_id,
        "dsl": dsl,
        "generated": written,
        "preview": preview(&dsl, today, 7, state.config.lock_time, state.clock.now()),
    }))
}

pub async fn maintenance(state: &AppState) -> Result<(), AppError> {
    let today = shanghai_date(state.clock.now());
    for offset in -1..=2 {
        let date = today + Duration::days(offset);
        persist_lock_if_needed(state, date).await?;
    }
    let accounts: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM account a
        WHERE EXISTS (SELECT 1 FROM shift_plan s WHERE s.account_id = a.id)
           OR EXISTS (SELECT 1 FROM meal_order m WHERE m.account_id = a.id)
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let range = horizon(state);
    for account_id in accounts {
        if load_template(state, account_id).await?.is_none() {
            continue;
        }
        generate_range(state, account_id, range.clone()).await?;
    }
    Ok(())
}

fn horizon(state: &AppState) -> std::ops::RangeInclusive<NaiveDate> {
    let today = shanghai_date(state.clock.now());
    today..=(today + Duration::days(GENERATE_HORIZON_DAYS))
}

pub async fn generate_range(
    state: &AppState,
    account_id: Uuid,
    dates: std::ops::RangeInclusive<NaiveDate>,
) -> Result<u32, AppError> {
    let Some((_, dsl)) = load_template(state, account_id).await? else {
        return Ok(0);
    };
    let eligible: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(SELECT 1 FROM shift_plan WHERE account_id = $1)
            OR EXISTS(SELECT 1 FROM meal_order WHERE account_id = $1)
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await?;
    if !eligible {
        return Ok(0);
    }
    let mut written = 0u32;
    let mut date = *dates.start();
    let end = *dates.end();
    while date <= end {
        if meal_is_locked(date, state.config.lock_time, state.clock.now()) {
            persist_lock_if_needed(state, date).await?;
        } else {
            for slot in SLOTS {
                if write_resolved(state, account_id, date, slot, &dsl, "dsl").await? {
                    written += 1;
                }
            }
        }
        date += Duration::days(1);
    }
    Ok(written)
}

async fn write_resolved(
    state: &AppState,
    account_id: Uuid,
    date: NaiveDate,
    slot: &str,
    dsl: &MealDsl,
    source: &str,
) -> Result<bool, AppError> {
    if meal_is_locked(date, state.config.lock_time, state.clock.now()) {
        return Ok(false);
    }
    let resolved = resolve_meal(dsl, date, slot);
    let existing: Option<(bool,)> = sqlx::query_as(
        r#"
        SELECT ordering FROM meal_order
        WHERE account_id = $1 AND meal_date = $2 AND slot = $3 AND locked_at IS NULL
        "#,
    )
    .bind(account_id)
    .bind(date)
    .bind(slot)
    .fetch_optional(&state.pool)
    .await?;
    let locked_row: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM meal_order
            WHERE account_id = $1 AND meal_date = $2 AND slot = $3 AND locked_at IS NOT NULL
        )
        "#,
    )
    .bind(account_id)
    .bind(date)
    .bind(slot)
    .fetch_one(&state.pool)
    .await?;
    if locked_row {
        return Ok(false);
    }
    if resolved.ordering {
        let Some(canteen_id) = resolved.canteen_id else {
            return Ok(false);
        };
        upsert_order(
            state,
            account_id,
            date,
            slot,
            Some(canteen_id),
            true,
            source,
        )
        .await?;
        return Ok(true);
    }
    if resolved.explicit || existing.is_some() {
        upsert_order(
            state,
            account_id,
            date,
            slot,
            resolved.canteen_id,
            false,
            source,
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

async fn upsert_order(
    state: &AppState,
    account_id: Uuid,
    date: NaiveDate,
    slot: &str,
    canteen_id: Option<Uuid>,
    ordering: bool,
    source: &str,
) -> Result<(), AppError> {
    let now = state.clock.now();
    sqlx::query(
        r#"
        INSERT INTO meal_order (account_id, meal_date, slot, canteen_id, ordering, source, submitted_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (account_id, meal_date, slot) DO UPDATE
        SET canteen_id = EXCLUDED.canteen_id,
            ordering = EXCLUDED.ordering,
            source = EXCLUDED.source,
            submitted_at = EXCLUDED.submitted_at
        WHERE meal_order.locked_at IS NULL
        "#,
    )
    .bind(account_id)
    .bind(date)
    .bind(slot)
    .bind(canteen_id)
    .bind(ordering)
    .bind(source)
    .bind(now)
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn persist_lock_if_needed(state: &AppState, date: NaiveDate) -> Result<(), AppError> {
    let now = state.clock.now();
    if !meal_is_locked(date, state.config.lock_time, now) {
        return Ok(());
    }
    sqlx::query(
        r#"
        INSERT INTO meal_day_lock (meal_date, locked_at) VALUES ($1, $2)
        ON CONFLICT (meal_date) DO NOTHING
        "#,
    )
    .bind(date)
    .bind(now)
    .execute(&state.pool)
    .await?;
    sqlx::query("UPDATE meal_order SET locked_at = $2 WHERE meal_date = $1 AND locked_at IS NULL")
        .bind(date)
        .bind(now)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn ensure_member(
    state: &AppState,
    account_id: Uuid,
    canteen_id: Uuid,
) -> Result<(), AppError> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM canteen_member WHERE account_id = $1 AND canteen_id = $2)",
    )
    .bind(account_id)
    .bind(canteen_id)
    .fetch_one(&state.pool)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::bad("canteen_unbound", "还没有绑定这个食堂"))
    }
}

async fn load_template(
    state: &AppState,
    account_id: Uuid,
) -> Result<Option<(Uuid, MealDsl)>, AppError> {
    let row = sqlx::query("SELECT id, dsl FROM meal_template WHERE account_id = $1")
        .bind(account_id)
        .fetch_optional(&state.pool)
        .await?;
    match row {
        None => Ok(None),
        Some(row) => {
            let dsl = serde_json::from_value(row.get("dsl")).map_err(|_| AppError::internal())?;
            Ok(Some((row.get("id"), dsl)))
        }
    }
}

async fn load_template_by_id(state: &AppState, id: Uuid) -> Result<MealDsl, AppError> {
    let row = sqlx::query("SELECT dsl FROM meal_template WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("template_not_found"))?;
    serde_json::from_value(row.get("dsl")).map_err(|_| AppError::internal())
}

async fn ensure_template_owner(
    state: &AppState,
    account_id: Uuid,
    template_id: Uuid,
) -> Result<(), AppError> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM meal_template WHERE id = $1 AND account_id = $2)",
    )
    .bind(template_id)
    .bind(account_id)
    .fetch_one(&state.pool)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::not_found("template_not_found"))
    }
}

async fn upsert_template(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    dsl: &MealDsl,
    now: chrono::DateTime<Utc>,
) -> Result<Uuid, AppError> {
    let doc = serde_json::to_value(dsl).map_err(|_| AppError::internal())?;
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO meal_template (account_id, dsl, updated_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (account_id) DO UPDATE SET dsl = EXCLUDED.dsl, updated_at = EXCLUDED.updated_at
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(doc)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

async fn sync_shift(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    dsl: &MealDsl,
    now: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    let kind = if dsl.mode == "work2_rest4" {
        "work2_rest4"
    } else {
        "custom"
    };
    sqlx::query(
        r#"
        INSERT INTO shift_plan (account_id, kind, anchor_date, updated_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (account_id) DO UPDATE
        SET kind = EXCLUDED.kind, anchor_date = EXCLUDED.anchor_date, updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(account_id)
    .bind(kind)
    .bind(dsl.anchor_date)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn preview(
    dsl: &MealDsl,
    start: NaiveDate,
    days: i64,
    lock_time: chrono::NaiveTime,
    now: chrono::DateTime<Utc>,
) -> Vec<Value> {
    (0..days)
        .map(|offset| {
            let date = start + Duration::days(offset);
            let locked = meal_is_locked(date, lock_time, now);
            let mut slots = serde_json::Map::new();
            for slot in SLOTS {
                let resolved = resolve_meal(dsl, date, slot);
                slots.insert(
                    (*slot).to_string(),
                    json!({
                        "ordering": resolved.ordering,
                        "color": if locked { "locked" } else { order_color(resolved.ordering) },
                        "canteen_id": resolved.canteen_id,
                        "locked": locked,
                    }),
                );
            }
            json!({"date": date, "locked": locked, "slots": slots})
        })
        .collect()
}

fn meal_after(state: &AppState, slot: &str) -> chrono::NaiveTime {
    match slot {
        "breakfast" => state.config.breakfast_after,
        "lunch" => state.config.lunch_after,
        _ => state.config.dinner_after,
    }
}

pub async fn home_board(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::MealSelfRead, None)?;
    let now = state.clock.now();
    let today = shanghai_date(now);
    let horizon_end = today + Duration::days(13);
    let rows = sqlx::query(
        r#"
        SELECT meal_date, slot, canteen_id, ordering, locked_at
        FROM meal_order
        WHERE account_id = $1 AND meal_date >= $2 AND meal_date <= $3
        "#,
    )
    .bind(user.account_id)
    .bind(today)
    .bind(horizon_end)
    .fetch_all(&state.pool)
    .await?;

    let mut next_hit: Option<(NaiveDate, String, Option<Uuid>)> = None;
    for offset in 0..14 {
        let date = today + Duration::days(offset);
        for slot in SLOTS {
            if now >= shanghai_at(date, meal_after(state, slot)) {
                continue;
            }
            let hit = rows.iter().find(|row| {
                row.get::<NaiveDate, _>("meal_date") == date
                    && row.get::<String, _>("slot") == *slot
                    && row.get::<bool, _>("ordering")
            });
            if let Some(row) = hit {
                next_hit = Some((date, (*slot).to_string(), row.get("canteen_id")));
                break;
            }
        }
        if next_hit.is_some() {
            break;
        }
    }

    let next = match next_hit {
        None => Value::Null,
        Some((date, slot, canteen_id)) => {
            let canteen_name: Option<String> = match canteen_id {
                Some(id) => {
                    sqlx::query_scalar("SELECT name FROM canteen WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&state.pool)
                        .await?
                }
                None => None,
            };
            let mut menu = json!({"菜": [], "饭": [], "汤": []});
            if let Some(id) = canteen_id {
                let dishes = sqlx::query(
                    r#"
                    SELECT d.name, d.kind
                    FROM menu_pick p
                    JOIN dish d ON d.id = p.dish_id
                    WHERE p.canteen_id = $1 AND p.meal_date = $2 AND p.slot = $3 AND d.enabled = TRUE
                    ORDER BY d.kind, d.name
                    "#,
                )
                .bind(id)
                .bind(date)
                .bind(&slot)
                .fetch_all(&state.pool)
                .await?;
                for dish in dishes {
                    let kind: String = dish.get("kind");
                    let name: String = dish.get("name");
                    if let Some(items) = menu.get_mut(&kind).and_then(Value::as_array_mut) {
                        items.push(json!(name));
                    }
                }
            }
            json!({
                "date": date,
                "slot": slot,
                "canteen_id": canteen_id,
                "canteen_name": canteen_name,
                "menu": menu,
            })
        }
    };

    let mut days = Vec::new();
    for offset in 0..7 {
        let date = today + Duration::days(offset);
        let locked = meal_is_locked(date, state.config.lock_time, now)
            || rows.iter().any(|row| {
                row.get::<NaiveDate, _>("meal_date") == date
                    && row
                        .get::<Option<chrono::DateTime<Utc>>, _>("locked_at")
                        .is_some()
            });
        let mut slots = serde_json::Map::new();
        for slot in SLOTS {
            let row = rows.iter().find(|row| {
                row.get::<NaiveDate, _>("meal_date") == date
                    && row.get::<String, _>("slot") == *slot
            });
            let state_name = if locked {
                "locked"
            } else if row.is_none() {
                "unset"
            } else if row.unwrap().get::<bool, _>("ordering") {
                "ordering"
            } else {
                "skip"
            };
            slots.insert((*slot).to_string(), json!({"state": state_name}));
        }
        days.push(json!({"date": date, "locked": locked, "slots": slots}));
    }

    Ok(json!({"next": next, "days": days}))
}

pub async fn bind_canteens(
    state: &AppState,
    user: &CurrentUser,
    canteen_ids: &[Uuid],
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MealSelfWrite, None)?;
    for id in canteen_ids {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM canteen WHERE id = $1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
        if !exists {
            return Err(AppError::bad("canteen_not_found", "食堂不存在"));
        }
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM canteen_member WHERE account_id = $1")
        .bind(user.account_id)
        .execute(&mut *tx)
        .await?;
    for id in canteen_ids {
        sqlx::query("INSERT INTO canteen_member (account_id, canteen_id) VALUES ($1, $2)")
            .bind(user.account_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    audit_user(
        state,
        user,
        &hit,
        "canteen_bind",
        Some("account"),
        Some(user.account_id.to_string()),
        json!({"count": canteen_ids.len()}),
    )
    .await?;
    Ok(json!({"canteen_ids": canteen_ids}))
}

pub async fn my_canteens(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT c.id, c.name
        FROM canteen_member m
        JOIN canteen c ON c.id = m.canteen_id
        WHERE m.account_id = $1
        ORDER BY c.name
        "#,
    )
    .bind(user.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "name": row.get::<String, _>("name"),
        })).collect::<Vec<_>>()
    }))
}

#[allow(dead_code)]
fn _utc() -> chrono::DateTime<Utc> {
    Utc::now()
}
