use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use minijinja::context;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    errors::{AppError, AppResult},
    middleware::{
        auth::{AuthUser, FerrisUser},
        csrf_guard::CsrfCtx,
    },
    services::dynamic_scoring as market,
    state::AppState,
    templates::UserView,
};

pub async fn page(
    State(state): State<Arc<AppState>>,
    FerrisUser(user): FerrisUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let mut tx = state.db.begin().await?;
    let settings = market::settings(&mut tx).await?;
    let rules = market::rules(&mut tx).await?;
    let at = market::now(&mut tx).await?;
    let prices = market::prices(&mut tx, &at).await?;
    let started: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM rounds) OR
        EXISTS(SELECT 1 FROM event_state WHERE started_at IS NOT NULL)",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(state
        .templates
        .render(
            "admin/scoring.html",
            context! {
                user => UserView::from(&user), csrf_token => ctx.token(&state.keys),
                settings => settings, rules => rules, challenges => prices, started => started,
            },
        )?
        .into_response())
}

#[derive(Deserialize)]
pub struct SettingsForm {
    pub action: String,
    pub reason: String,
    pub revision: i64,
    pub enabled: Option<i64>,
    pub difficulty: Option<String>,
    pub starting_points: Option<i64>,
    pub minimum_points: Option<i64>,
    pub maximum_points: Option<i64>,
    pub quiet_minutes: Option<i64>,
    pub recovery_per_minute: Option<i64>,
    pub drop_percent: Option<i64>,
}

fn required<T>(value: Option<T>) -> AppResult<T> {
    value.ok_or_else(|| AppError::BadRequest("Missing scoring setting".into()))
}

pub async fn save(
    State(state): State<Arc<AppState>>,
    FerrisUser(user): FerrisUser,
    Form(form): Form<SettingsForm>,
) -> AppResult<Response> {
    match form.action.as_str() {
        "mode" => {
            let enabled = required(form.enabled)?;
            if !matches!(enabled, 0 | 1) {
                return Err(AppError::BadRequest("Invalid scoring mode".into()));
            }
            market::set_mode(
                &state.db,
                enabled == 1,
                form.revision,
                &user.user_id,
                &form.reason,
            )
            .await?;
        }
        "rule" => {
            let rule = market::Rule {
                difficulty: required(form.difficulty)?,
                starting_points: required(form.starting_points)?,
                minimum_points: required(form.minimum_points)?,
                maximum_points: required(form.maximum_points)?,
                quiet_minutes: required(form.quiet_minutes)?,
                recovery_per_minute: required(form.recovery_per_minute)?,
                drop_percent: required(form.drop_percent)?,
            };
            market::update_rule(&state.db, &rule, form.revision, &user.user_id, &form.reason)
                .await?;
        }
        _ => return Err(AppError::BadRequest("Unknown scoring action".into())),
    }
    Ok(Redirect::to("/admin/scoring").into_response())
}

#[derive(Deserialize)]
pub struct PointsForm {
    pub points: i64,
    pub reason: String,
}

pub async fn override_points(
    State(state): State<Arc<AppState>>,
    FerrisUser(user): FerrisUser,
    Path(id): Path<String>,
    Form(form): Form<PointsForm>,
) -> AppResult<Response> {
    market::override_points(&state.db, &id, form.points, &user.user_id, &form.reason).await?;
    Ok(Redirect::to("/admin/scoring").into_response())
}

#[derive(Serialize)]
pub struct PlayerPrice {
    pub id: String,
    pub current_points: i64,
    pub effective_points: i64,
    pub points_awarded: Option<i64>,
}

/// Authorized, read-only quotes; never expose rules, other pools, or flag fields.
pub async fn player_prices(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
) -> AppResult<Response> {
    let team = user
        .team_id
        .ok_or_else(|| AppError::Forbidden("Join a team first".into()))?;
    let mut tx = state.db.begin().await?;
    let at = market::now(&mut tx).await?;
    let enabled = market::settings(&mut tx).await?.enabled;
    let rows = sqlx::query(
        "SELECT c.id, p.repeat_penalty, s.points_awarded,
        (SELECT COALESCE(SUM(h.point_cost), 0) FROM hint_usage hu JOIN hints h ON h.id = hu.hint_id
            WHERE hu.team_id = p.team_id AND h.challenge_id = c.id) AS hint_cost
        FROM team_challenge_pool p JOIN challenges c ON c.id = p.challenge_id
        JOIN teams t ON t.id = p.team_id JOIN rounds r ON r.round_number = p.round_number
        LEFT JOIN solves s ON s.team_id = p.team_id AND s.challenge_id = c.id
        WHERE p.team_id = ? AND c.is_active = 1 AND t.is_locked = 1 AND t.disbanded = 0
          AND r.ended_at IS NULL AND (r.auto_end_at IS NULL OR r.auto_end_at > ?)",
    )
    .bind(&team)
    .bind(&at)
    .fetch_all(&mut *tx)
    .await?;
    let prices: HashMap<_, _> = market::prices(&mut tx, &at)
        .await?
        .into_iter()
        .map(|p| (p.id, p.current_points))
        .collect();
    let mut quotes = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let gross = prices[&id];
        let repeat: i64 = row.try_get("repeat_penalty")?;
        let hints: i64 = row.try_get("hint_cost")?;
        quotes.push(PlayerPrice {
            id,
            current_points: gross,
            effective_points: gross.saturating_sub(repeat).saturating_sub(hints).max(0),
            points_awarded: row.try_get("points_awarded")?,
        });
    }
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"dynamic_scoring": enabled, "server_time": at, "challenges": quotes}),
    )
    .into_response())
}

#[cfg(test)]
mod tests;
