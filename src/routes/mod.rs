use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/health", get(health))
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
