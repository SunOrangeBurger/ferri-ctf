pub mod admin;
pub mod auth;
pub mod pages;

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(pages::index))
        .route("/health", get(health))
        .route("/static/*path", get(crate::templates::static_file))
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route("/register", get(auth::register_page).post(auth::register_submit))
        .route("/logout", post(auth::logout))
        .route("/admin/login", get(auth::admin_login_page).post(auth::admin_login_submit))
        .route("/admin/dashboard", get(admin::dashboard))
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
