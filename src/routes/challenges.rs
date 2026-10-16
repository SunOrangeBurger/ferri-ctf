use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use minijinja::context;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;

use crate::{
    errors::{AppError, AppResult},
    middleware::{auth::AuthUser, csrf_guard::CsrfCtx},
    ratelimit,
    services::{
        files as files_svc, hints as hints_svc, rounds, dynamic_scoring,
    },
    state::AppState,
    templates::UserView,
};

#[derive(Deserialize)]
pub struct SubmitFlagForm {
    pub flag: String,
}

#[derive(Deserialize)]
pub struct RequestHintForm {
    pub hint_id: String,
}

#[derive(Deserialize)]
pub struct ChallengeQuery {
    pub status: Option<String>,
}

#[derive(Serialize)]
pub struct PooledChallengeCard {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub difficulty: String,
    pub base_points: i64,
    pub repeat_penalty: i64,
    pub hint_cost: i64,
    pub effective_points: i64,
    pub has_file: bool,
    pub is_solved: bool,
    pub points_awarded: Option<i64>,
}

/// GET /challenges — Pooled challenges for current round
pub async fn challenges_list(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let Some(ref team_id) = user.team_id else {
        return Ok(Redirect::to("/onboarding").into_response());
    };

    // Verify team is locked
    let is_locked: bool = sqlx::query_scalar(
        "SELECT is_locked FROM teams WHERE id = ? AND disbanded = 0",
    )
    .bind(team_id)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(false);

    if !is_locked {
        return Ok(Redirect::to("/team").into_response());
    }

    let active_round = rounds::get_active_round(&state.db).await?;
    let u_view = UserView::from(&user);

    let Some(ref round) = active_round else {
        let html = state.templates.render(
            "challenges/list.html",
            context! {
                csrf_token => ctx.token(&state.keys),
                user => u_view,
                challenges => Vec::<PooledChallengeCard>::new(),
                active_round => None::<crate::models::Round>,
                is_between_rounds => true,
            },
        )?;
        return Ok(html.into_response());
    };

    let rows = sqlx::query(
        "SELECT c.id, c.title, c.description, c.category, c.difficulty,
                c.points AS base_points, p.repeat_penalty,
                (c.file_path IS NOT NULL) AS has_file,
                (s.solved_at IS NOT NULL) AS is_solved, s.points_awarded,
                (SELECT COALESCE(SUM(h.point_cost), 0)
                 FROM hint_usage hu JOIN hints h ON h.id = hu.hint_id
                 WHERE hu.team_id = p.team_id AND h.challenge_id = c.id) AS hint_cost
         FROM team_challenge_pool p
         JOIN challenges c ON c.id = p.challenge_id
         LEFT JOIN solves s ON s.team_id = p.team_id AND s.challenge_id = c.id
         WHERE p.team_id = ? AND p.round_number = ? AND c.is_active = 1
         ORDER BY (CASE c.difficulty WHEN 'easy' THEN 1 WHEN 'medium' THEN 2 WHEN 'hard' THEN 3 ELSE 4 END) ASC,
                  c.points ASC",
    )
    .bind(team_id)
    .bind(round.round_number)
    .fetch_all(&state.db)
    .await?;

    let mut price_tx = state.db.begin().await?;
    let dynamic = dynamic_scoring::settings(&mut price_tx).await?.enabled;
    let at = dynamic_scoring::now(&mut price_tx).await?;
    let price_map: HashMap<_, _> = dynamic_scoring::prices(&mut price_tx, &at).await?
        .into_iter().map(|p| (p.id, p.current_points)).collect();
    price_tx.commit().await?;

    let mut challenges = Vec::with_capacity(rows.len());
    for r in rows {
        let cid: String = r.try_get("id")?;
        // A challenge can be deleted by the operator between the pool query and
        // the price snapshot. Omit it instead of indexing a missing quote.
        let Some(&base_pts) = price_map.get(&cid) else { continue; };
        let repeat_penalty: i64 = r.try_get("repeat_penalty")?;
        let hint_cost: i64 = r.try_get("hint_cost")?;
        let effective = std::cmp::max(0, base_pts - repeat_penalty - hint_cost);

        challenges.push(PooledChallengeCard {
            id: cid,
            title: r.try_get("title")?,
            description: r.try_get("description")?,
            category: r.try_get("category")?,
            difficulty: r.try_get("difficulty")?,
            base_points: base_pts,
            repeat_penalty,
            hint_cost,
            effective_points: effective,
            has_file: r.try_get("has_file")?,
            is_solved: r.try_get("is_solved")?,
            points_awarded: r.try_get("points_awarded")?,
        });
    }

    let html = state.templates.render(
        "challenges/list.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            user => u_view,
            challenges => challenges,
            active_round => active_round,
            is_between_rounds => false,
            dynamic_scoring => dynamic,
        },
    )?;

    Ok(html.into_response())
}

#[derive(Serialize)]
pub struct RoundViewStub {}

/// GET /challenges/:id — Challenge detail
pub async fn challenge_detail(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
    Path(challenge_id): Path<String>,
    Query(query): Query<ChallengeQuery>,
) -> AppResult<Response> {
    let Some(ref team_id) = user.team_id else {
        return Ok(Redirect::to("/onboarding").into_response());
    };

    let active_round = rounds::get_active_round(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("No active round".into()))?;

    // Challenge must be active and in team pool for active round
    let row = sqlx::query(
        "SELECT c.id, c.title, c.description, c.category, c.difficulty,
                c.points AS base_points, p.repeat_penalty,
                (c.file_path IS NOT NULL) AS has_file, c.file_name, c.challenge_url,
                (s.solved_at IS NOT NULL) AS is_solved, s.points_awarded
         FROM team_challenge_pool p
         JOIN challenges c ON c.id = p.challenge_id
         JOIN teams t ON t.id = p.team_id
         LEFT JOIN solves s ON s.team_id = p.team_id AND s.challenge_id = c.id
         WHERE p.team_id = ? AND c.id = ? AND p.round_number = ?
           AND c.is_active = 1 AND t.is_locked = 1 AND t.disbanded = 0",
    )
    .bind(team_id)
    .bind(&challenge_id)
    .bind(active_round.round_number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Challenge not in current pool".into()))?;

    let mut price_tx = state.db.begin().await?;
    let dynamic = dynamic_scoring::settings(&mut price_tx).await?.enabled;
    let at = dynamic_scoring::now(&mut price_tx).await?;
    let base_pts = dynamic_scoring::prices(&mut price_tx, &at).await?.into_iter()
        .find(|p| p.id == challenge_id).ok_or_else(|| AppError::NotFound("Challenge".into()))?.current_points;
    price_tx.commit().await?;
    let repeat_penalty: i64 = row.try_get("repeat_penalty")?;
    let is_solved: bool = row.try_get("is_solved")?;
    let points_awarded: Option<i64> = row.try_get("points_awarded")?;

    let hints = hints_svc::get_hints_for_challenge(&state.db, &challenge_id, team_id).await?;
    let hint_cost = hints_svc::team_hint_cost_for_challenge(&state.db, team_id, &challenge_id).await?;
    let effective = std::cmp::max(0, base_pts - repeat_penalty - hint_cost);

    let u_view = UserView::from(&user);
    let html = state.templates.render(
        "challenges/detail.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            user => u_view,
            challenge => context! {
                id => row.try_get::<String, _>("id")?,
                title => row.try_get::<String, _>("title")?,
                description => row.try_get::<String, _>("description")?,
                category => row.try_get::<String, _>("category")?,
                difficulty => row.try_get::<String, _>("difficulty")?,
                base_points => base_pts,
                repeat_penalty => repeat_penalty,
                hint_cost => hint_cost,
                effective_points => effective,
                has_file => row.try_get::<bool, _>("has_file")?,
                file_name => row.try_get::<Option<String>, _>("file_name")?,
                challenge_url => row.try_get::<Option<String>, _>("challenge_url")?,
                is_solved => is_solved,
                points_awarded => points_awarded,
            },
            hints => hints,
            status_msg => query.status,
            dynamic_scoring => dynamic,
            active_round => active_round,
        },
    )?;

    Ok(html.into_response())
}

/// GET /challenges/:id/download — Stream zip attachment
pub async fn download_challenge_file(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    Path(challenge_id): Path<String>,
) -> AppResult<Response> {
    let Some(ref team_id) = user.team_id else {
        return Err(AppError::NotFound("Not found".into()));
    };

    if !state
        .limiter
        .check("chal_download", team_id, ratelimit::CHALLENGE_DOWNLOAD)
    {
        return Err(AppError::TooManyRequests);
    }

    let record = sqlx::query(
        "SELECT c.file_path, c.file_name
         FROM challenges c
         JOIN team_challenge_pool p ON p.challenge_id = c.id
         JOIN teams t ON t.id = p.team_id
         JOIN rounds r ON r.round_number = p.round_number
         WHERE p.team_id = ? AND c.id = ? AND c.is_active = 1 AND t.is_locked = 1
           AND r.ended_at IS NULL",
    )
    .bind(team_id)
    .bind(&challenge_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Not found".into()))?;

    let file_path: Option<String> = record.try_get("file_path")?;
    let path_str = file_path.ok_or_else(|| AppError::NotFound("No file attached".into()))?;
    let path = std::path::PathBuf::from(path_str);

    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| AppError::NotFound("File not found on server".into()))?;

    let len = file.metadata().await.map(|m| m.len()).ok();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));

    let file_name: Option<String> = record.try_get("file_name")?;
    let safe_name = files_svc::sanitize_header_filename(
        &file_name.unwrap_or_else(|| "challenge.zip".into()),
    );
    let disposition = format!("attachment; filename=\"{safe_name}\"");

    let mut resp = Response::new(body);
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition)?,
    );
    if let Some(l) = len {
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(l));
    }

    Ok(resp)
}

/// POST /challenges/:id/submit — Submit flag
pub async fn submit_flag(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    Path(challenge_id): Path<String>,
    Form(form): Form<SubmitFlagForm>,
) -> AppResult<Response> {
    let Some(ref team_id) = user.team_id else {
        return Err(AppError::Forbidden("You must be in a team to submit flags".into()));
    };

    if !state
        .limiter
        .check("chal_submit", team_id, ratelimit::CHALLENGE_SUBMIT)
    {
        return Err(AppError::TooManyRequests);
    }

    let status = dynamic_scoring::submit(&state, team_id, &user.user_id, &challenge_id, &form.flag).await?;
    Ok(Redirect::to(&format!("/challenges/{challenge_id}?status={status}")).into_response())
}

/// POST /challenges/:id/hint — Request hint unlock
pub async fn request_hint_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    Path(challenge_id): Path<String>,
    Form(form): Form<RequestHintForm>,
) -> AppResult<Response> {
    let Some(ref team_id) = user.team_id else {
        return Err(AppError::Forbidden("You must be in a team to request hints".into()));
    };

    hints_svc::request_hint(&state.db, team_id, &form.hint_id).await?;

    Ok(Redirect::to(&format!("/challenges/{challenge_id}?status=hint_unlocked")).into_response())
}
