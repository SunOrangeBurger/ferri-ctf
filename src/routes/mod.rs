pub mod admin;
pub mod auth;
pub mod challenges;
pub mod onboarding;
pub mod pages;
pub mod profile;
pub mod scoreboard;
pub mod team;

use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Public routes
        .route("/", get(pages::index))
        .route("/health", get(health))
        .route("/static/*path", get(crate::templates::static_file))
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route("/register", get(auth::register_page).post(auth::register_submit))
        .route("/logout", post(auth::logout))
        .route("/scoreboard", get(scoreboard::scoreboard_page))
        .route("/api/scoreboard", get(scoreboard::scoreboard_json))
        .route("/admin/login", get(auth::admin_login_page).post(auth::admin_login_submit))

        // Onboarding routes
        .route("/onboarding", get(onboarding::onboarding_page))
        .route("/team/create", post(onboarding::create_team_submit))
        .route("/team/join", post(onboarding::join_team_submit))
        .route("/team/preview/:code", get(onboarding::preview_team))

        // Team management routes
        .route("/team", get(team::team_dashboard))
        .route("/team/leave", post(team::leave_team_submit))
        .route("/team/kick", post(team::kick_member_submit))
        .route("/team/transfer", post(team::transfer_captain_submit))
        .route("/team/rename", post(team::rename_team_submit))
        .route("/team/disband", post(team::disband_team_submit))

        // Challenges & solves routes
        .route("/challenges", get(challenges::challenges_list))
        .route("/challenges/:id", get(challenges::challenge_detail))
        .route("/challenges/:id/download", get(challenges::download_challenge_file))
        .route("/challenges/:id/submit", post(challenges::submit_flag))
        .route("/challenges/:id/hint", post(challenges::request_hint_submit))

        // Profile routes
        .route("/profile", get(profile::profile_page))
        .route("/profile/password", post(profile::change_password_submit))

        // Admin-level routes
        .route("/admin/dashboard", get(admin::dashboard))
        .route("/admin/teams", get(admin::teams_list))
        .route("/admin/teams/:id/action", post(admin::team_action))
        .route("/admin/teams/:id/score", post(admin::adjust_team_score))
        .route("/admin/users/:id/action", post(admin::user_action))
        .route("/admin/export/:kind", get(admin::export_csv))
        .route("/admin/audit", get(admin::audit_log_page))

        // Ferris-level routes
        .route("/admin/event/start", post(admin::start_event_submit))
        .route("/admin/rounds", get(admin::rounds_page))
        .route("/admin/rounds/start", post(admin::start_next_round_submit))
        .route("/admin/rounds/end", post(admin::end_round_submit))
        .route("/admin/rounds/limit", post(admin::set_round_limit_submit))
        .route("/admin/challenges", get(admin::challenges_admin_page))
        .route(
            "/admin/challenges/new",
            post(admin::create_challenge_submit).layer(DefaultBodyLimit::max(35 * 1024 * 1024)),
        )
        .route("/admin/challenges/:id/edit", post(admin::edit_challenge_submit))
        .route("/admin/challenges/:id/toggle", post(admin::toggle_challenge_submit))
        .route("/admin/challenges/bulk", post(admin::bulk_challenge_submit))
        .route("/admin/challenges/:id/delete", post(admin::delete_challenge_submit))
        .route("/admin/challenges/:id/hints", post(admin::add_hint_submit))
        .route("/admin/hints/:id/edit", post(admin::edit_hint_submit))
        .route("/admin/hints/:id/delete", post(admin::delete_hint_submit))
        .route("/admin/admins", get(admin::admins_page))
        .route("/admin/admins/add", post(admin::add_admin_submit))
        .route("/admin/admins/:id/remove", post(admin::remove_admin_submit))
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(&state.db).await.is_ok();
    let status = if db_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (
        status,
        Json(json!({
            "status": if db_ok { "ok" } else { "degraded" },
            "db": if db_ok { "ok" } else { "error" },
            "uptime_seconds": state.started.elapsed().as_secs(),
        })),
    )
}
