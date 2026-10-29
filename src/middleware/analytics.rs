use std::{sync::Arc, time::Instant};

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::state::{AppState, RequestMetric, MAX_METRICS_REQUESTS};

/// Middleware that tracks latency and response status codes for non-static HTTP requests.
/// Records metrics into an in-memory rolling window of the last 10,000 requests.
pub async fn track_analytics(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();

    // Ignore static file requests to avoid skewing metrics
    if path.starts_with("/static") {
        return next.run(req).await;
    }

    let start = Instant::now();
    let resp = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = resp.status().as_u16();

    // Record into in-memory rolling store
    {
        let mut store = state.metrics.write().unwrap_or_else(|e| e.into_inner());
        if store.len() >= MAX_METRICS_REQUESTS {
            store.pop_front();
        }
        store.push_back(RequestMetric {
            path,
            status,
            duration_ms,
        });
    }

    resp
}
