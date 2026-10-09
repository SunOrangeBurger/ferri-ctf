use std::sync::Arc;

use axum::{extract::State, response::{IntoResponse, Response}};
use minijinja::context;

use crate::{
    errors::AppResult,
    middleware::{auth::OptionalUser, csrf_guard::CsrfCtx},
    state::AppState,
    templates::UserView,
};

pub async fn index(
    State(state): State<Arc<AppState>>,
    OptionalUser(user): OptionalUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    // Only logged-in pages carry a form (the logout button), so only they ask for a token.
    let token = user.as_ref().map(|_| ctx.token(&state.keys));
    let page = state.templates.render(
        "index.html",
        context! { user => user.as_ref().map(UserView::from), csrf_token => token },
    )?;
    Ok(page.into_response())
}
