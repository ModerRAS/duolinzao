use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Duration, NaiveDate};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use super::audit_user;
use crate::crypto::sniff_image;
use crate::domain::{
    empty_groups, meal_is_locked, next_slot, push_group, score_window_open, shanghai_date,
    tone_from_mean, valid_kind, valid_slot, week_start, KINDS, SLOTS,
};
use crate::error::AppError;
use crate::extract::CurrentUser;
use crate::rbac::{authorize, can_see_strong_alert, AuthHit, Perm};
use crate::state::AppState;

pub fn image_url(dish_id: Uuid, image_path: Option<&str>) -> Option<String> {
    image_path
        .filter(|path| !path.is_empty())
        .map(|_| format!("/api/dishes/{dish_id}/image"))
}

pub async fn list_dishes(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    kind: Option<&str>,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::MenuRead, Some(canteen_id))?;
    if let Some(kind) = kind {
        if !valid_kind(kind) {
            return Err(AppError::bad("kind_invalid", "类别只能是菜、饭、汤"));
        }
    }
    let rows = sqlx::query(
        r#"
        SELECT id, name, kind, taste_tags, image_path, enabled
        FROM dish
        WHERE canteen_id = $1 AND ($2::text IS NULL OR kind = $2)
        ORDER BY name
        "#,
    )
    .bind(canteen_id)
    .bind(kind)
    .fetch_all(&state.pool)
    .await?;
    let mut groups = empty_groups();
    for row in rows {
        let id: Uuid = row.get("id");
        let kind: String = row.get("kind");
        push_group(
            &mut groups,
            &kind,
            json!({
                "id": id,
                "name": row.get::<String, _>("name"),
                "kind": kind,
                "taste_tags": row.get::<Vec<String>, _>("taste_tags"),
                "image_url": image_url(id, row.get::<Option<String>, _>("image_path").as_deref()),
                "enabled": row.get::<bool, _>("enabled"),
                "canteen_id": canteen_id,
            }),
        );
    }
    Ok(json!({"groups": groups}))
}

pub struct DishInput {
    pub name: String,
    pub kind: String,
    pub taste_tags: Vec<String>,
    pub enabled: Option<bool>,
    pub canteen_id: Option<Uuid>,
}

fn clean_tags(tags: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut clean = Vec::new();
    for tag in tags {
        let tag = tag.trim().to_string();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > 16 {
            return Err(AppError::bad("tag_invalid", "口味标签过长"));
        }
        if !clean.contains(&tag) {
            clean.push(tag);
        }
    }
    if clean.len() > 12 {
        return Err(AppError::bad("tag_invalid", "口味标签过多"));
    }
    Ok(clean)
}

pub async fn create_dish(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    input: DishInput,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MenuWrite, Some(canteen_id))?;
    let kind = require_kind(&input.kind)?;
    let name = require_name(&input.name)?;
    let tags = clean_tags(input.taste_tags)?;
    let enabled = input.enabled.unwrap_or(true);
    let now = state.clock.now();
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO dish (canteen_id, name, kind, taste_tags, enabled, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $6)
        RETURNING id
        "#,
    )
    .bind(canteen_id)
    .bind(&name)
    .bind(&kind)
    .bind(&tags)
    .bind(enabled)
    .bind(now)
    .fetch_one(&state.pool)
    .await?;
    audit_user(
        state,
        user,
        &hit,
        "dish_create",
        Some("dish"),
        Some(id.to_string()),
        json!({"kind": kind, "name": name}),
    )
    .await?;
    Ok(json!({
        "id": id,
        "name": name,
        "kind": kind,
        "taste_tags": tags,
        "image_url": null,
        "enabled": enabled,
        "canteen_id": canteen_id,
    }))
}

pub async fn update_dish(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    dish_id: Uuid,
    input: DishInput,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MenuWrite, Some(canteen_id))?;
    let current = load_dish(state, dish_id).await?;
    if current.canteen_id != canteen_id {
        return Err(AppError::not_found("dish_not_found"));
    }
    let target = input.canteen_id.unwrap_or(canteen_id);
    if target != canteen_id {
        let _ = authorize(&user.assignments, Perm::MenuWrite, Some(target))?;
    }
    let kind = require_kind(&input.kind)?;
    let name = require_name(&input.name)?;
    let tags = clean_tags(input.taste_tags)?;
    let enabled = input.enabled.unwrap_or(current.enabled);
    sqlx::query(
        r#"
        UPDATE dish
        SET canteen_id = $1, name = $2, kind = $3, taste_tags = $4, enabled = $5, updated_at = $6
        WHERE id = $7
        "#,
    )
    .bind(target)
    .bind(&name)
    .bind(&kind)
    .bind(&tags)
    .bind(enabled)
    .bind(state.clock.now())
    .bind(dish_id)
    .execute(&state.pool)
    .await?;
    audit_user(
        state,
        user,
        &hit,
        "dish_update",
        Some("dish"),
        Some(dish_id.to_string()),
        json!({"kind": kind, "enabled": enabled, "name": name}),
    )
    .await?;
    Ok(json!({
        "id": dish_id,
        "name": name,
        "kind": kind,
        "taste_tags": tags,
        "image_url": image_url(dish_id, current.image_path.as_deref()),
        "enabled": enabled,
        "canteen_id": target,
    }))
}

pub async fn save_image(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    dish_id: Uuid,
    bytes: &[u8],
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MenuWrite, Some(canteen_id))?;
    let dish = load_dish(state, dish_id).await?;
    if dish.canteen_id != canteen_id {
        return Err(AppError::not_found("dish_not_found"));
    }
    if bytes.len() as u64 > state.config.dish_image_max_bytes {
        return Err(AppError::bad("image_too_large", "图片超过大小上限"));
    }
    let Some((content_type, ext)) = sniff_image(bytes) else {
        return Err(AppError::bad("image_invalid", "只接受 JPEG、PNG 或 WebP"));
    };
    let filename = format!("{dish_id}.{ext}");
    let path = PathBuf::from(&state.config.dish_image_dir).join(&filename);
    if let Some(old) = &dish.image_path {
        if old != &filename {
            let _ =
                tokio::fs::remove_file(PathBuf::from(&state.config.dish_image_dir).join(old)).await;
        }
    }
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|_| AppError::internal())?;
    let replaced = dish.image_path.is_some();
    sqlx::query("UPDATE dish SET image_path = $1, updated_at = $2 WHERE id = $3")
        .bind(&filename)
        .bind(state.clock.now())
        .bind(dish_id)
        .execute(&state.pool)
        .await?;
    audit_user(
        state,
        user,
        &hit,
        "dish_image",
        Some("dish"),
        Some(dish_id.to_string()),
        json!({"image": if replaced { "replaced" } else { "uploaded" }}),
    )
    .await?;
    Ok(json!({
        "image_url": image_url(dish_id, Some(&filename)),
        "content_type": content_type,
    }))
}

pub async fn remove_image(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    dish_id: Uuid,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::MenuWrite, Some(canteen_id))?;
    let dish = load_dish(state, dish_id).await?;
    if dish.canteen_id != canteen_id {
        return Err(AppError::not_found("dish_not_found"));
    }
    if let Some(old) = &dish.image_path {
        let _ = tokio::fs::remove_file(PathBuf::from(&state.config.dish_image_dir).join(old)).await;
    }
    sqlx::query("UPDATE dish SET image_path = NULL, updated_at = $1 WHERE id = $2")
        .bind(state.clock.now())
        .bind(dish_id)
        .execute(&state.pool)
        .await?;
    audit_user(
        state,
        user,
        &hit,
        "dish_image",
        Some("dish"),
        Some(dish_id.to_string()),
        json!({"image": "removed"}),
    )
    .await?;
    Ok(json!({"image_url": null}))
}

pub async fn read_image(
    state: &AppState,
    user: &CurrentUser,
    dish_id: Uuid,
) -> Result<(Vec<u8>, &'static str), AppError> {
    let dish = load_dish(state, dish_id).await?;
    let Some(path) = dish.image_path.as_deref() else {
        return Err(AppError::not_found("image_not_found"));
    };
    if !can_view_image(state, user, &dish).await? {
        return Err(AppError::forbidden());
    }
    if path.contains('/') || path.contains("..") {
        return Err(AppError::not_found("image_not_found"));
    }
    let bytes = tokio::fs::read(PathBuf::from(&state.config.dish_image_dir).join(path))
        .await
        .map_err(|_| AppError::not_found("image_not_found"))?;
    let content_type = sniff_image(&bytes)
        .map(|item| item.0)
        .unwrap_or("application/octet-stream");
    Ok((bytes, content_type))
}

async fn can_view_image(
    state: &AppState,
    user: &CurrentUser,
    dish: &DishRow,
) -> Result<bool, AppError> {
    if authorize(&user.assignments, Perm::MenuRead, Some(dish.canteen_id)).is_ok()
        || authorize(
            &user.assignments,
            Perm::OptimizerRead,
            Some(dish.canteen_id),
        )
        .is_ok()
        || authorize(
            &user.assignments,
            Perm::RatingAdminRead,
            Some(dish.canteen_id),
        )
        .is_ok()
    {
        return Ok(true);
    }
    if authorize(&user.assignments, Perm::RatingSelfWrite, None).is_err() {
        return Ok(false);
    }
    let rows = sqlx::query(
        r#"
        SELECT meal_date, slot FROM meal_order
        WHERE account_id = $1 AND canteen_id = $2 AND ordering = TRUE
        "#,
    )
    .bind(user.account_id)
    .bind(dish.canteen_id)
    .fetch_all(&state.pool)
    .await?;
    let now = state.clock.now();
    for row in rows {
        let date: NaiveDate = row.get("meal_date");
        let slot: String = row.get("slot");
        let after = slot_after(state, &slot);
        if !score_window_open(date, after, now) {
            continue;
        }
        let picked: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM menu_pick
                WHERE canteen_id = $1 AND meal_date = $2 AND slot = $3 AND dish_id = $4
            )
            "#,
        )
        .bind(dish.canteen_id)
        .bind(date)
        .bind(&slot)
        .bind(dish.id)
        .fetch_one(&state.pool)
        .await?;
        if picked {
            return Ok(true);
        }
    }
    Ok(false)
}

fn slot_after(state: &AppState, slot: &str) -> chrono::NaiveTime {
    match slot {
        "breakfast" => state.config.breakfast_after,
        "lunch" => state.config.lunch_after,
        _ => state.config.dinner_after,
    }
}

#[allow(dead_code)]
struct DishRow {
    id: Uuid,
    canteen_id: Uuid,
    name: String,
    kind: String,
    taste_tags: Vec<String>,
    image_path: Option<String>,
    enabled: bool,
}

async fn load_dish(state: &AppState, id: Uuid) -> Result<DishRow, AppError> {
    let row = sqlx::query(
        "SELECT id, canteen_id, name, kind, taste_tags, image_path, enabled FROM dish WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("dish_not_found"))?;
    Ok(DishRow {
        id: row.get("id"),
        canteen_id: row.get("canteen_id"),
        name: row.get("name"),
        kind: row.get("kind"),
        taste_tags: row.get("taste_tags"),
        image_path: row.get("image_path"),
        enabled: row.get("enabled"),
    })
}

fn require_kind(kind: &str) -> Result<String, AppError> {
    if valid_kind(kind) {
        Ok(kind.to_string())
    } else {
        Err(AppError::bad("kind_invalid", "类别只能是菜、饭、汤"))
    }
}

fn require_name(name: &str) -> Result<String, AppError> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(AppError::bad("name_invalid", "菜品名称无效"));
    }
    Ok(name)
}

struct Candidate {
    id: Uuid,
    name: String,
    kind: String,
    taste_tags: Vec<String>,
    image_path: Option<String>,
    selected: bool,
    mean: Option<f64>,
}

fn grouped_candidates(items: Vec<Candidate>) -> Value {
    let mut groups = empty_groups();
    let mut by_kind: HashMap<String, Vec<Candidate>> = HashMap::new();
    for item in items {
        by_kind.entry(item.kind.clone()).or_default().push(item);
    }
    for kind in KINDS {
        let mut list = by_kind.remove(kind).unwrap_or_default();
        list.sort_by(|a, b| match (a.mean, b.mean) {
            (Some(left), Some(right)) => right
                .partial_cmp(&left)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        });
        for item in list {
            let tone = tone_from_mean(item.mean);
            push_group(
                &mut groups,
                kind,
                json!({
                    "dish_id": item.id,
                    "name": item.name,
                    "kind": item.kind,
                    "taste_tags": item.taste_tags,
                    "image_url": image_url(item.id, item.image_path.as_deref()),
                    "selected": item.selected,
                    "tone": tone,
                }),
            );
        }
    }
    json!(groups)
}

async fn preference_means(
    state: &AppState,
    diners: &[Uuid],
    dishes: &[Uuid],
) -> Result<HashMap<Uuid, f64>, AppError> {
    if diners.is_empty() || dishes.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT dish_id, AVG(mean)::float8 AS mean
        FROM preference
        WHERE account_id = ANY($1) AND dish_id = ANY($2)
        GROUP BY dish_id
        "#,
    )
    .bind(diners)
    .bind(dishes)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.get("dish_id"), row.get("mean")))
        .collect())
}

async fn diners(
    state: &AppState,
    canteen_id: Uuid,
    date: NaiveDate,
    slot: &str,
) -> Result<Vec<Uuid>, AppError> {
    sqlx::query_scalar(
        r#"
        SELECT account_id FROM meal_order
        WHERE canteen_id = $1 AND meal_date = $2 AND slot = $3 AND ordering = TRUE
        "#,
    )
    .bind(canteen_id)
    .bind(date)
    .bind(slot)
    .fetch_all(&state.pool)
    .await
    .map_err(Into::into)
}

pub async fn selection_slot(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    date: NaiveDate,
    slot: &str,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::OptimizerRead, Some(canteen_id))?;
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    let dishes = sqlx::query(
        r#"
        SELECT id, name, kind, taste_tags, image_path
        FROM dish
        WHERE canteen_id = $1 AND enabled = TRUE
        "#,
    )
    .bind(canteen_id)
    .fetch_all(&state.pool)
    .await?;
    let picked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT dish_id FROM menu_pick WHERE canteen_id = $1 AND meal_date = $2 AND slot = $3",
    )
    .bind(canteen_id)
    .bind(date)
    .bind(slot)
    .fetch_all(&state.pool)
    .await?;
    let ids: Vec<Uuid> = dishes.iter().map(|row| row.get("id")).collect();
    let people = diners(state, canteen_id, date, slot).await?;
    let means = preference_means(state, &people, &ids).await?;
    let items = dishes
        .into_iter()
        .map(|row| {
            let id: Uuid = row.get("id");
            Candidate {
                id,
                name: row.get("name"),
                kind: row.get("kind"),
                taste_tags: row.get("taste_tags"),
                image_path: row.get("image_path"),
                selected: picked.contains(&id),
                mean: means.get(&id).copied(),
            }
        })
        .collect();
    let (next_date, next_slot_name) = next_slot(date, slot);
    Ok(json!({
        "date": date,
        "slot": slot,
        "expected_headcount": people.len(),
        "groups": grouped_candidates(items),
        "next": {"date": next_date, "slot": next_slot_name},
    }))
}

pub async fn put_selection(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    date: NaiveDate,
    slot: &str,
    dish_ids: &[Uuid],
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::OptimizerRead, Some(canteen_id))?;
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    for id in dish_ids {
        let row = sqlx::query("SELECT canteen_id, enabled FROM dish WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::bad("dish_invalid", "只能选择本食堂当前启用的餐品"))?;
        if row.get::<Uuid, _>("canteen_id") != canteen_id || !row.get::<bool, _>("enabled") {
            return Err(AppError::bad(
                "dish_invalid",
                "只能选择本食堂当前启用的餐品",
            ));
        }
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM menu_pick WHERE canteen_id = $1 AND meal_date = $2 AND slot = $3")
        .bind(canteen_id)
        .bind(date)
        .bind(slot)
        .execute(&mut *tx)
        .await?;
    for id in dish_ids {
        sqlx::query(
            "INSERT INTO menu_pick (canteen_id, meal_date, slot, dish_id) VALUES ($1, $2, $3, $4)",
        )
        .bind(canteen_id)
        .bind(date)
        .bind(slot)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    audit_user(
        state,
        user,
        &hit,
        "menu_pick",
        Some("menu_pick"),
        Some(format!("{canteen_id}/{date}/{slot}")),
        json!({"count": dish_ids.len()}),
    )
    .await?;
    selection_slot(state, user, canteen_id, date, slot).await
}

pub async fn selection_week(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    date: NaiveDate,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::OptimizerRead, Some(canteen_id))?;
    let start = week_start(date);
    let end = start + Duration::days(6);
    let picks = sqlx::query(
        r#"
        SELECT p.meal_date, p.slot, d.id, d.name, d.kind, d.taste_tags, d.image_path
        FROM menu_pick p
        JOIN dish d ON d.id = p.dish_id
        WHERE p.canteen_id = $1 AND p.meal_date >= $2 AND p.meal_date <= $3
        "#,
    )
    .bind(canteen_id)
    .bind(start)
    .bind(end)
    .fetch_all(&state.pool)
    .await?;
    let orders = sqlx::query(
        r#"
        SELECT meal_date, slot, account_id
        FROM meal_order
        WHERE canteen_id = $1 AND meal_date >= $2 AND meal_date <= $3 AND ordering = TRUE
        "#,
    )
    .bind(canteen_id)
    .bind(start)
    .bind(end)
    .fetch_all(&state.pool)
    .await?;
    let dish_ids: Vec<Uuid> = picks.iter().map(|row| row.get("id")).collect();
    let diner_ids: Vec<Uuid> = orders.iter().map(|row| row.get("account_id")).collect();
    let pref_rows = if diner_ids.is_empty() || dish_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query(
            r#"
            SELECT account_id, dish_id, mean
            FROM preference
            WHERE account_id = ANY($1) AND dish_id = ANY($2)
            "#,
        )
        .bind(&diner_ids)
        .bind(&dish_ids)
        .fetch_all(&state.pool)
        .await?
    };
    let mut days = Vec::new();
    for offset in 0..7 {
        let day = start + Duration::days(offset);
        let mut slots = serde_json::Map::new();
        for slot in SLOTS {
            let people = orders
                .iter()
                .filter(|row| {
                    row.get::<NaiveDate, _>("meal_date") == day
                        && row.get::<String, _>("slot") == *slot
                })
                .map(|row| row.get::<Uuid, _>("account_id"))
                .collect::<Vec<_>>();
            let items = picks
                .iter()
                .filter(|row| {
                    row.get::<NaiveDate, _>("meal_date") == day
                        && row.get::<String, _>("slot") == *slot
                })
                .map(|row| {
                    let id: Uuid = row.get("id");
                    let samples: Vec<f64> = pref_rows
                        .iter()
                        .filter(|pref| {
                            pref.get::<Uuid, _>("dish_id") == id
                                && people.contains(&pref.get::<Uuid, _>("account_id"))
                        })
                        .map(|pref| pref.get::<f64, _>("mean"))
                        .collect();
                    let mean = if samples.is_empty() {
                        None
                    } else {
                        Some(samples.iter().sum::<f64>() / samples.len() as f64)
                    };
                    Candidate {
                        id,
                        name: row.get("name"),
                        kind: row.get("kind"),
                        taste_tags: row.get("taste_tags"),
                        image_path: row.get("image_path"),
                        selected: true,
                        mean,
                    }
                })
                .collect();
            slots.insert(
                (*slot).to_string(),
                json!({
                    "expected_headcount": people.len(),
                    "groups": grouped_candidates(items),
                }),
            );
        }
        days.push(json!({"date": day, "slots": slots}));
    }
    Ok(json!({"week_start": start, "days": days}))
}

pub async fn headcount(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    date: NaiveDate,
) -> Result<Value, AppError> {
    let _ = crate::rbac::authorize_any(
        &user.assignments,
        &[Perm::MenuRead, Perm::OptimizerRead, Perm::MealAdminOverride],
        Some(canteen_id),
    )?;
    crate::ops::meal::persist_lock_if_needed(state, date).await?;
    let rows = sqlx::query(
        r#"
        SELECT slot, COUNT(*)::bigint AS count
        FROM meal_order
        WHERE canteen_id = $1 AND meal_date = $2 AND ordering = TRUE
        GROUP BY slot
        "#,
    )
    .bind(canteen_id)
    .bind(date)
    .fetch_all(&state.pool)
    .await?;
    let mut slots = serde_json::Map::new();
    for slot in SLOTS {
        slots.insert((*slot).to_string(), json!(0));
    }
    for row in rows {
        slots.insert(
            row.get::<String, _>("slot"),
            json!(row.get::<i64, _>("count")),
        );
    }
    Ok(json!({
        "date": date,
        "locked": meal_is_locked(date, state.config.lock_time, state.clock.now()),
        "slots": slots,
    }))
}

pub async fn submit_score(
    state: &AppState,
    user: &CurrentUser,
    date: NaiveDate,
    slot: &str,
    dish_id: Uuid,
    value: Option<i16>,
    comment: Option<String>,
    anonymous: bool,
) -> Result<Value, AppError> {
    let hit = authorize(&user.assignments, Perm::RatingSelfWrite, None)?;
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    let comment = comment.and_then(|text| {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let Some(value) = value else {
        if comment.is_some() {
            return Err(AppError::bad("comment_only", "不能只写评语"));
        }
        return Err(AppError::bad("score_required", "分数必填"));
    };
    if !matches!(value, -2 | -1 | 0 | 1 | 2) {
        return Err(AppError::bad(
            "score_out_of_range",
            "分数只能是 -2 到 2 的整数",
        ));
    }
    if let Some(text) = &comment {
        if text.chars().count() > 300 {
            return Err(AppError::bad("comment_too_long", "评语最长 300 字"));
        }
    }
    let order = sqlx::query(
        r#"
        SELECT canteen_id, ordering FROM meal_order
        WHERE account_id = $1 AND meal_date = $2 AND slot = $3
        "#,
    )
    .bind(user.account_id)
    .bind(date)
    .bind(slot)
    .fetch_optional(&state.pool)
    .await?;
    let Some(order) = order else {
        return Err(AppError::bad("score_not_ordered", "只能给自己报过的餐打分"));
    };
    if !order.get::<bool, _>("ordering") {
        return Err(AppError::bad("score_not_ordered", "只能给自己报过的餐打分"));
    }
    let Some(canteen_id) = order.get::<Option<Uuid>, _>("canteen_id") else {
        return Err(AppError::bad("score_not_ordered", "只能给自己报过的餐打分"));
    };
    let picked: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM menu_pick
            WHERE canteen_id = $1 AND meal_date = $2 AND slot = $3 AND dish_id = $4
        )
        "#,
    )
    .bind(canteen_id)
    .bind(date)
    .bind(slot)
    .bind(dish_id)
    .fetch_one(&state.pool)
    .await?;
    if !picked {
        return Err(AppError::bad("score_not_served", "这道餐品不在当餐选定里"));
    }
    let now = state.clock.now();
    if !score_window_open(date, slot_after(state, slot), now) {
        return Err(AppError::bad(
            "score_window_closed",
            "不在餐后一周的评分窗口内",
        ));
    }
    let exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM score
            WHERE account_id = $1 AND meal_date = $2 AND slot = $3 AND dish_id = $4
        )
        "#,
    )
    .bind(user.account_id)
    .bind(date)
    .bind(slot)
    .bind(dish_id)
    .fetch_one(&state.pool)
    .await?;
    if exists {
        return Err(AppError::conflict(
            "score_exists",
            "这道餐品已经评过，不能改分",
        ));
    }
    let mut tx = state.pool.begin().await?;
    let score_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO score (account_id, meal_date, slot, dish_id, value, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(user.account_id)
    .bind(date)
    .bind(slot)
    .bind(dish_id)
    .bind(value)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(body) = &comment {
        sqlx::query(
            r#"
            INSERT INTO comment (score_id, body, anonymous, created_at)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(score_id)
        .bind(body)
        .bind(anonymous)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO preference (account_id, dish_id, mean, sample_count, updated_at)
        SELECT $1, $2, AVG(value)::float8, COUNT(*)::int, $3
        FROM score
        WHERE account_id = $1 AND dish_id = $2
        ON CONFLICT (account_id, dish_id) DO UPDATE
        SET mean = EXCLUDED.mean, sample_count = EXCLUDED.sample_count, updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(user.account_id)
    .bind(dish_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    refresh_alert(state, dish_id).await?;
    audit_user(
        state,
        user,
        &hit,
        "score",
        Some("score"),
        Some(score_id.to_string()),
        json!({"value": value, "has_comment": comment.is_some(), "anonymous": anonymous}),
    )
    .await?;
    Ok(json!({"id": score_id, "value": value}))
}

async fn refresh_alert(state: &AppState, dish_id: Uuid) -> Result<(), AppError> {
    let dish = load_dish(state, dish_id).await?;
    let row = sqlx::query(
        r#"
        SELECT COALESCE(AVG(value), 0)::float8 AS mean, COUNT(DISTINCT account_id)::bigint AS samples
        FROM score WHERE dish_id = $1
        "#,
    )
    .bind(dish_id)
    .fetch_one(&state.pool)
    .await?;
    let mean: f64 = row.get("mean");
    let samples: i64 = row.get("samples");
    let (threshold, min_samples) = super::alert_config(state).await?;
    if samples < min_samples || mean >= threshold {
        return Ok(());
    }
    let exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM notification
            WHERE kind = 'strong_alert' AND payload->>'dish_id' = $1
        )
        "#,
    )
    .bind(dish_id.to_string())
    .fetch_one(&state.pool)
    .await?;
    if exists {
        return Ok(());
    }
    let staff: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT account_id FROM role_assignment
        WHERE role = 'canteen' AND $1 = ANY(canteen_ids)
        "#,
    )
    .bind(dish.canteen_id)
    .fetch_all(&state.pool)
    .await?;
    let now = state.clock.now();
    let message = "样本已足够，均分低于当前阈值";
    for account_id in staff {
        sqlx::query(
            r#"
            INSERT INTO notification (account_id, kind, title, body, payload, created_at)
            VALUES ($1, 'strong_alert', '强提醒', $2, $3, $4)
            "#,
        )
        .bind(account_id)
        .bind(message)
        .bind(json!({"dish_id": dish_id, "canteen_id": dish.canteen_id, "kind": dish.kind}))
        .bind(now)
        .execute(&state.pool)
        .await?;
    }
    let enabled: bool = sqlx::query_scalar("SELECT enabled FROM dish WHERE id = $1")
        .bind(dish_id)
        .fetch_one(&state.pool)
        .await?;
    if enabled != dish.enabled {
        return Err(AppError::internal());
    }
    Ok(())
}

pub async fn score_targets(
    state: &AppState,
    user: &CurrentUser,
    date: NaiveDate,
    slot: &str,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::RatingSelfWrite, None)?;
    if !valid_slot(slot) {
        return Err(AppError::bad("slot_invalid", "餐次只有早、中、晚"));
    }
    let open = score_window_open(date, slot_after(state, slot), state.clock.now());
    let order = sqlx::query(
        r#"
        SELECT canteen_id, ordering FROM meal_order
        WHERE account_id = $1 AND meal_date = $2 AND slot = $3
        "#,
    )
    .bind(user.account_id)
    .bind(date)
    .bind(slot)
    .fetch_optional(&state.pool)
    .await?;
    let mut groups = empty_groups();
    let canteen_id = order
        .as_ref()
        .filter(|row| row.get::<bool, _>("ordering"))
        .and_then(|row| row.get::<Option<Uuid>, _>("canteen_id"));
    if open {
        if let Some(canteen_id) = canteen_id {
            let rows = sqlx::query(
                r#"
                SELECT d.id, d.name, d.kind, d.taste_tags, d.image_path,
                       EXISTS(
                           SELECT 1 FROM score s
                           WHERE s.account_id = $4 AND s.meal_date = p.meal_date
                             AND s.slot = p.slot AND s.dish_id = d.id
                       ) AS scored
                FROM menu_pick p
                JOIN dish d ON d.id = p.dish_id
                WHERE p.canteen_id = $1 AND p.meal_date = $2 AND p.slot = $3
                ORDER BY d.name
                "#,
            )
            .bind(canteen_id)
            .bind(date)
            .bind(slot)
            .bind(user.account_id)
            .fetch_all(&state.pool)
            .await?;
            for row in rows {
                let id: Uuid = row.get("id");
                let kind: String = row.get("kind");
                push_group(
                    &mut groups,
                    &kind,
                    json!({
                        "dish_id": id,
                        "name": row.get::<String, _>("name"),
                        "kind": kind,
                        "taste_tags": row.get::<Vec<String>, _>("taste_tags"),
                        "image_url": image_url(id, row.get::<Option<String>, _>("image_path").as_deref()),
                        "scored": row.get::<bool, _>("scored"),
                    }),
                );
            }
        }
    }
    Ok(json!({
        "date": date,
        "slot": slot,
        "window_open": open,
        "groups": groups,
    }))
}

pub async fn score_summaries(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    kind: Option<&str>,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::RatingAdminRead, Some(canteen_id))?;
    if let Some(kind) = kind {
        if !valid_kind(kind) {
            return Err(AppError::bad("kind_invalid", "类别只能是菜、饭、汤"));
        }
    }
    let rows = sqlx::query(
        r#"
        SELECT d.id, d.name, d.kind, d.taste_tags, d.image_path,
               COALESCE(AVG(s.value), 0)::float8 AS visible_mean,
               COUNT(s.id)::bigint AS sample_count,
               COUNT(s.id) FILTER (WHERE s.value = -2) AS c_m2,
               COUNT(s.id) FILTER (WHERE s.value = -1) AS c_m1,
               COUNT(s.id) FILTER (WHERE s.value = 0) AS c_0,
               COUNT(s.id) FILTER (WHERE s.value = 1) AS c_1,
               COUNT(s.id) FILTER (WHERE s.value = 2) AS c_2
        FROM dish d
        LEFT JOIN score s ON s.dish_id = d.id
        WHERE d.canteen_id = $1 AND ($2::text IS NULL OR d.kind = $2)
        GROUP BY d.id
        ORDER BY d.name
        "#,
    )
    .bind(canteen_id)
    .bind(kind)
    .fetch_all(&state.pool)
    .await?;
    let mut groups = empty_groups();
    for row in rows {
        let id: Uuid = row.get("id");
        let kind: String = row.get("kind");
        push_group(
            &mut groups,
            &kind,
            json!({
                "dish_id": id,
                "name": row.get::<String, _>("name"),
                "kind": kind,
                "taste_tags": row.get::<Vec<String>, _>("taste_tags"),
                "image_url": image_url(id, row.get::<Option<String>, _>("image_path").as_deref()),
                "visible_mean": if row.get::<i64, _>("sample_count") == 0 {
                    Value::Null
                } else {
                    json!(row.get::<f64, _>("visible_mean"))
                },
                "sample_count": row.get::<i64, _>("sample_count"),
                "counts": {
                    "-2": row.get::<i64, _>("c_m2"),
                    "-1": row.get::<i64, _>("c_m1"),
                    "0": row.get::<i64, _>("c_0"),
                    "1": row.get::<i64, _>("c_1"),
                    "2": row.get::<i64, _>("c_2"),
                },
            }),
        );
    }
    Ok(json!({"groups": groups}))
}

pub async fn comments(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
    kind: Option<&str>,
) -> Result<Value, AppError> {
    let _ = authorize(&user.assignments, Perm::CommentAdminRead, Some(canteen_id))?;
    if let Some(kind) = kind {
        if !valid_kind(kind) {
            return Err(AppError::bad("kind_invalid", "类别只能是菜、饭、汤"));
        }
    }
    let rows = sqlx::query(
        r#"
        SELECT c.id, c.body, c.anonymous, s.meal_date, s.slot, d.name AS dish_name, d.kind,
               a.username, a.display_name
        FROM comment c
        JOIN score s ON s.id = c.score_id
        JOIN dish d ON d.id = s.dish_id
        JOIN account a ON a.id = s.account_id
        WHERE d.canteen_id = $1 AND c.deleted_at IS NULL
          AND ($2::text IS NULL OR d.kind = $2)
        ORDER BY c.created_at DESC
        "#,
    )
    .bind(canteen_id)
    .bind(kind)
    .fetch_all(&state.pool)
    .await?;
    let mut groups = empty_groups();
    for row in rows {
        let kind: String = row.get("kind");
        let anonymous: bool = row.get("anonymous");
        push_group(
            &mut groups,
            &kind,
            json!({
                "comment_id": row.get::<Uuid, _>("id"),
                "body": row.get::<String, _>("body"),
                "anonymous": anonymous,
                "meal_date": row.get::<NaiveDate, _>("meal_date"),
                "slot": row.get::<String, _>("slot"),
                "dish_name": row.get::<String, _>("dish_name"),
                "kind": kind,
                "username": if anonymous { Value::Null } else { json!(row.get::<String, _>("username")) },
                "display_name": if anonymous {
                    Value::Null
                } else {
                    json!(row.get::<Option<String>, _>("display_name"))
                },
            }),
        );
    }
    Ok(json!({"groups": groups}))
}

pub async fn alerts(
    state: &AppState,
    user: &CurrentUser,
    canteen_id: Uuid,
) -> Result<Value, AppError> {
    let _ = can_see_strong_alert(&user.assignments, canteen_id)?;
    let (threshold, min_samples) = super::alert_config(state).await?;
    let rows = sqlx::query(
        r#"
        SELECT d.id, d.name, d.kind, d.enabled,
               AVG(s.value)::float8 AS visible_mean,
               COUNT(DISTINCT s.account_id)::bigint AS sample_count
        FROM dish d
        JOIN score s ON s.dish_id = d.id
        WHERE d.canteen_id = $1
        GROUP BY d.id
        HAVING COUNT(DISTINCT s.account_id) >= $2 AND AVG(s.value) < $3
        ORDER BY d.name
        "#,
    )
    .bind(canteen_id)
    .bind(min_samples)
    .bind(threshold)
    .fetch_all(&state.pool)
    .await?;
    let mut groups = empty_groups();
    for row in rows {
        let kind: String = row.get("kind");
        push_group(
            &mut groups,
            &kind,
            json!({
                "dish_id": row.get::<Uuid, _>("id"),
                "name": row.get::<String, _>("name"),
                "kind": kind,
                "sample_count": row.get::<i64, _>("sample_count"),
                "visible_mean": row.get::<f64, _>("visible_mean"),
                "threshold": threshold,
                "enabled": row.get::<bool, _>("enabled"),
                "message": "样本已足够，均分低于当前阈值",
            }),
        );
    }
    Ok(json!({"groups": groups, "threshold": threshold, "min_samples": min_samples}))
}

pub async fn notifications(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT id, kind, title, body, payload, read_at, created_at
        FROM notification
        WHERE account_id = $1
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .bind(user.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(json!({
        "items": rows.iter().map(|row| json!({
            "id": row.get::<Uuid, _>("id"),
            "kind": row.get::<String, _>("kind"),
            "title": row.get::<String, _>("title"),
            "body": row.get::<String, _>("body"),
            "payload": row.get::<Value, _>("payload"),
            "read_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("read_at"),
            "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        })).collect::<Vec<_>>()
    }))
}

pub async fn read_notification(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
) -> Result<Value, AppError> {
    let updated = sqlx::query(
        "UPDATE notification SET read_at = $1 WHERE id = $2 AND account_id = $3 AND read_at IS NULL",
    )
    .bind(state.clock.now())
    .bind(id)
    .bind(user.account_id)
    .execute(&state.pool)
    .await?;
    if updated.rows_affected() == 0 {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM notification WHERE id = $1 AND account_id = $2)",
        )
        .bind(id)
        .bind(user.account_id)
        .fetch_one(&state.pool)
        .await?;
        if !exists {
            return Err(AppError::not_found("notification_not_found"));
        }
    }
    Ok(json!({"id": id, "read": true}))
}

#[allow(dead_code)]
fn _hit(hit: AuthHit) -> AuthHit {
    hit
}

#[allow(dead_code)]
fn _today(state: &AppState) -> NaiveDate {
    shanghai_date(state.clock.now())
}
