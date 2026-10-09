use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use minijinja::context;
use serde::Deserialize;

use crate::{
    errors::{AppError, AppResult},
    middleware::{auth::AuthUser, csrf_guard::CsrfCtx},
    ratelimit::{self, ClientIp},
    services::team as team_svc,
    state::AppState,
    templates::UserView,
};

#[derive(Deserialize)]
pub struct CreateRoomForm {
    pub name: String,
}

#[derive(Deserialize)]
pub struct JoinRoomForm {
    pub code: String,
}

/// GET /onboarding
pub async fn onboarding_page(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    // If user already has a team, go straight to team dashboard
    if user.team_id.is_some() {
        return Ok(Redirect::to("/team").into_response());
    }

    let u_view = UserView::from(&user);
    let html = state.templates.render(
        "onboarding/index.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            user => u_view,
            error => None::<String>,
        },
    )?;

    Ok(html.into_response())
}

/// POST /team/create
pub async fn create_team_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
    Form(form): Form<CreateRoomForm>,
) -> AppResult<Response> {
    if user.team_id.is_some() {
        return Ok(Redirect::to("/team").into_response());
    }

    match team_svc::create_team(
        &state.db,
        &user.user_id,
        &form.name,
        state.cfg.grace_period_minutes,
    )
    .await
    {
        Ok(_) => Ok(Redirect::to("/team").into_response()),
        Err(e) => {
            let u_view = UserView::from(&user);
            let msg = match &e {
                AppError::BadRequest(m) | AppError::Conflict(m) | AppError::Forbidden(m) => {
                    m.clone()
                }
                _ => "Failed to create room".to_string(),
            };
            let html = state.templates.render(
                "onboarding/index.html",
                context! {
                    csrf_token => ctx.token(&state.keys),
                    user => u_view,
                    error => Some(msg),
                    team_name => form.name,
                },
            )?;
            Ok((StatusCode::BAD_REQUEST, html).into_response())
        }
    }
}

/// POST /team/join
pub async fn join_team_submit(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
    Form(form): Form<JoinRoomForm>,
) -> AppResult<Response> {
    if user.team_id.is_some() {
        return Ok(Redirect::to("/team").into_response());
    }

    if !state
        .limiter
        .check("team_join", &ip, ratelimit::TEAM_JOIN_PREVIEW)
    {
        return Err(AppError::TooManyRequests);
    }

    match team_svc::join_team(
        &state.db,
        &user.user_id,
        &form.code,
        state.cfg.grace_period_minutes,
    )
    .await
    {
        Ok(_) => Ok(Redirect::to("/team").into_response()),
        Err(e) => {
            let u_view = UserView::from(&user);
            let msg = match &e {
                AppError::BadRequest(m)
                | AppError::Conflict(m)
                | AppError::Forbidden(m)
                | AppError::NotFound(m) => m.clone(),
                _ => "Failed to join room".to_string(),
            };
            let html = state.templates.render(
                "onboarding/index.html",
                context! {
                    csrf_token => ctx.token(&state.keys),
                    user => u_view,
                    error => Some(msg),
                    join_code => form.code,
                },
            )?;
            Ok((StatusCode::BAD_REQUEST, html).into_response())
        }
    }
}

/// GET /team/preview/:code
pub async fn preview_team(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
    Path(code): Path<String>,
) -> AppResult<Response> {
    if !state
        .limiter
        .check("team_join", &ip, ratelimit::TEAM_JOIN_PREVIEW)
    {
        return Err(AppError::TooManyRequests);
    }

    let preview = team_svc::preview_team_by_code(&state.db, &code).await?;
    let u_view = UserView::from(&user);

    let html = state.templates.render(
        "onboarding/preview.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            user => u_view,
            team => preview.team,
            members => preview.members,
        },
    )?;

    Ok(html.into_response())
}
