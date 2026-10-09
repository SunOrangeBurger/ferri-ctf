use std::sync::Arc;

use axum::{extract::State, response::{IntoResponse, Response}};
use minijinja::context;

use crate::{
    errors::AppResult,
    middleware::{auth::AdminUser, csrf_guard::CsrfCtx},
    state::AppState,
    templates::UserView,
};

/// Placeholder landing page for admins; real content comes with the admin routes.
pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let page = state.templates.render(
        "admin/dashboard.html",
        context! { user => UserView::from(&user), csrf_token => ctx.token(&state.keys) },
    )?;
    Ok(page.into_response())
}
