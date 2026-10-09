use std::sync::Arc;

use axum::{middleware::from_fn_with_state, Router};

use crate::{
    middleware::{self, response},
    routes,
    state::AppState,
};

/// The complete application. Layer order, outermost first:
///   security_headers -> error_pages -> load_session -> csrf_guard -> admin_gate -> routes
pub fn build(state: Arc<AppState>) -> Router {
    middleware::secure(routes::router(), state.clone())
        .layer(from_fn_with_state(state.clone(), response::error_pages))
        .layer(from_fn_with_state(state, response::security_headers))
}
