use std::sync::Arc;

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use minijinja::context;
use serde_json::json;
use serde::Serialize;

use crate::{
    errors::AppResult,
    middleware::auth::OptionalUser,
    services::{rounds, scoring},
    state::AppState,
    templates::UserView,
};

/// Public scoreboard data excludes scores and all solve/performance metadata.
#[derive(Serialize)]
struct PublicScoreboardEntry {
    rank: usize,
    team_name: String,
    members: Vec<String>,
}

fn public_entries(entries: Vec<scoring::ScoreboardEntry>) -> Vec<PublicScoreboardEntry> {
    entries.into_iter().map(|entry| PublicScoreboardEntry {
        rank: entry.rank,
        team_name: entry.team_name,
        members: entry.members,
    }).collect()
}

/// GET /scoreboard — HTML leaderboard
pub async fn scoreboard_page(
    State(state): State<Arc<AppState>>,
    OptionalUser(user): OptionalUser,
) -> AppResult<Response> {
    let scoreboard = public_entries(scoring::get_scoreboard(&state).await?);
    let active_round = rounds::get_active_round(&state.db).await?;
    let u_view = user.as_ref().map(UserView::from);

    let html = state.templates.render(
        "scoreboard.html",
        context! {
            user => u_view,
            scoreboard => scoreboard,
            active_round => active_round,
        },
    )?;

    Ok(html.into_response())
}

/// GET /api/scoreboard — JSON scoreboard for polling
pub async fn scoreboard_json(
    State(state): State<Arc<AppState>>,
) -> AppResult<Response> {
    let scoreboard = public_entries(scoring::get_scoreboard(&state).await?);
    let active_round = rounds::get_active_round(&state.db).await?;

    Ok(Json(json!({
        "status": "ok",
        "active_round": active_round.map(|r| r.round_number),
        "scoreboard": scoreboard,
    }))
    .into_response())
}
