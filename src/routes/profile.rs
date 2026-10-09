use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Form,
};
use minijinja::context;
use serde::Deserialize;
use sqlx::Row;

use crate::{
    errors::{AppError, AppResult},
    middleware::{auth::AuthUser, csrf_guard::CsrfCtx},
    services::auth as auth_svc,
    state::AppState,
    templates::UserView,
};

#[derive(Deserialize)]
pub struct ChangePasswordForm {
    pub current_password: String,
    pub new_password: String,
    pub new_password_confirm: String,
}

/// GET /profile
pub async fn profile_page(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    render_profile(&state, &user, &ctx, StatusCode::OK, None, None).await
}

/// POST /profile/password
pub async fn change_password_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
    Form(form): Form<ChangePasswordForm>,
) -> AppResult<Response> {
    if form.new_password != form.new_password_confirm {
        return render_profile(
            &state,
            &user,
            &ctx,
            StatusCode::BAD_REQUEST,
            Some("New passwords do not match"),
            None,
        )
        .await;
    }

    match auth_svc::change_password(
        &state,
        &user.user_id,
        &form.current_password,
        &form.new_password,
        &user.session_id,
    )
    .await
    {
        Ok(_) => {
            render_profile(
                &state,
                &user,
                &ctx,
                StatusCode::OK,
                None,
                Some("Password changed successfully! Other active sessions have been logged out."),
            )
            .await
        }
        Err(e) => {
            let msg = match &e {
                AppError::BadRequest(m) => m.clone(),
                _ => "Failed to change password".to_string(),
            };
            render_profile(
                &state,
                &user,
                &ctx,
                StatusCode::BAD_REQUEST,
                Some(&msg),
                None,
            )
            .await
        }
    }
}

async fn render_profile(
    state: &AppState,
    user: &crate::session::SessionUser,
    ctx: &CsrfCtx,
    status: StatusCode,
    error: Option<&str>,
    success: Option<&str>,
) -> AppResult<Response> {
    let team_info = sqlx::query(
        "SELECT t.name AS team_name, tm.role
         FROM team_members tm
         JOIN teams t ON t.id = tm.team_id
         WHERE tm.user_id = ? AND t.disbanded = 0",
    )
    .bind(&user.user_id)
    .fetch_optional(&state.db)
    .await?;

    let (team_name, role) = match team_info {
        Some(r) => (r.try_get("team_name")?, r.try_get("role")?),
        None => ("None".to_string(), "None".to_string()),
    };

    let solves_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM solves WHERE user_id = ?",
    )
    .bind(&user.user_id)
    .fetch_one(&state.db)
    .await?;

    let u_view = UserView::from(user);
    let html = state.templates.render(
        "profile.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            user => u_view,
            team_name => team_name,
            role => role,
            solves_count => solves_count,
            error => error,
            success => success,
        },
    )?;

    Ok((status, html).into_response())
}
