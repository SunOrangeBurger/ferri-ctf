use std::sync::Arc;

use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use minijinja::context;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    errors::AppResult,
    middleware::{auth::AuthUser, csrf_guard::CsrfCtx},
    services::{rounds, scoring, team as team_svc},
    state::AppState,
    templates::UserView,
};

#[derive(Deserialize)]
pub struct TargetUserForm {
    pub user_id: String,
}

#[derive(Deserialize)]
pub struct RenameTeamForm {
    pub name: String,
}

#[derive(Serialize)]
pub struct TeamSolveItem {
    pub challenge_id: String,
    pub title: String,
    pub category: String,
    pub difficulty: String,
    pub points_awarded: i64,
    pub solved_at: String,
    pub solver_name: String,
}

/// GET /team — Team dashboard
pub async fn team_dashboard(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let team_info = team_svc::get_team_for_user(&state.db, &user.user_id).await?;
    let Some(team_with_members) = team_info else {
        return Ok(Redirect::to("/onboarding").into_response());
    };

    let team = team_with_members.team;
    let members = team_with_members.members;
    let is_captain = team.captain_id == user.user_id;

    // Fetch team solve history
    let solve_rows = sqlx::query(
        "SELECT s.challenge_id, c.title, c.category, c.difficulty, s.points_awarded, s.solved_at, u.display_name AS solver_name
         FROM solves s
         JOIN challenges c ON c.id = s.challenge_id
         JOIN users u ON u.id = s.user_id
         WHERE s.team_id = ?
         ORDER BY s.solved_at DESC",
    )
    .bind(&team.id)
    .fetch_all(&state.db)
    .await?;

    let mut solves = Vec::with_capacity(solve_rows.len());
    for r in solve_rows {
        solves.push(TeamSolveItem {
            challenge_id: r.try_get("challenge_id")?,
            title: r.try_get("title")?,
            category: r.try_get("category")?,
            difficulty: r.try_get("difficulty")?,
            points_awarded: r.try_get("points_awarded")?,
            solved_at: r.try_get("solved_at")?,
            solver_name: r.try_get("solver_name")?,
        });
    }

    // Current rank and total score from scoreboard
    let scoreboard = scoring::get_scoreboard(&state).await?;
    let rank_entry = scoreboard.iter().find(|e| e.team_id == team.id);
    let (current_rank, total_score) = match rank_entry {
        Some(e) => (Some(e.rank), e.total_score),
        None => (None, 0),
    };

    // Active round info
    let active_round = rounds::get_active_round(&state.db).await?;

    let u_view = UserView::from(&user);
    let html = state.templates.render(
        "team/dashboard.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            user => u_view,
            team => team,
            members => members,
            is_captain => is_captain,
            solves => solves,
            current_rank => current_rank,
            total_score => total_score,
            active_round => active_round,
        },
    )?;

    Ok(html.into_response())
}

/// POST /team/leave
pub async fn leave_team_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
) -> AppResult<Response> {
    team_svc::leave_team(&state.db, &user.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/onboarding").into_response())
}

/// POST /team/kick
pub async fn kick_member_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    Form(form): Form<TargetUserForm>,
) -> AppResult<Response> {
    team_svc::kick_member(&state.db, &user.user_id, &form.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/team").into_response())
}

/// POST /team/transfer
pub async fn transfer_captain_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    Form(form): Form<TargetUserForm>,
) -> AppResult<Response> {
    team_svc::transfer_captain(&state.db, &user.user_id, &form.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/team").into_response())
}

/// POST /team/rename
pub async fn rename_team_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    Form(form): Form<RenameTeamForm>,
) -> AppResult<Response> {
    team_svc::rename_team(&state.db, &user.user_id, &form.name).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/team").into_response())
}

/// POST /team/disband
pub async fn disband_team_submit(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
) -> AppResult<Response> {
    team_svc::disband_team(&state.db, &user.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/onboarding").into_response())
}
