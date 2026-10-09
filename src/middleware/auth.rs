use std::sync::Arc;

use axum::{
    async_trait,
    extract::{FromRequestParts, Request, State},
    http::request::Parts,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;

use crate::{
    errors::AppError,
    session::{self, SessionUser},
    state::AppState,
};

/// Inserted into request extensions by `load_session` on every request.
/// Extensions are server-side only; a client can never supply this.
#[derive(Clone, Debug)]
pub struct CurrentUser(pub Option<SessionUser>);

/// Outermost layer: cookie -> user. A DB error fails the request (500), never "anonymous".
pub async fn load_session(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let user = match jar.get(session::COOKIE_NAME) {
        None => None,
        Some(c) => match session::authenticate(&state.db, &state.keys, c.value()).await {
            Ok(u) => u,
            Err(e) => return e.into_response(),
        },
    };
    req.extensions_mut().insert(CurrentUser(user));
    next.run(req).await
}

fn current(parts: &Parts) -> Result<Option<&SessionUser>, AppError> {
    parts
        .extensions
        .get::<CurrentUser>()
        .map(|c| c.0.as_ref())
        .ok_or_else(|| AppError::internal("session middleware not installed"))
}

/// Never rejects for being logged out.
pub struct OptionalUser(pub Option<SessionUser>);

/// Logged-in users only. Browsers are redirected to /login.
pub struct AuthUser(pub SessionUser);

/// Admin or Ferris. Anyone else gets the standard 404 (spec 9.3).
pub struct AdminUser(pub SessionUser);

/// Ferris only. Anyone else gets the standard 404.
pub struct FerrisUser(pub SessionUser);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for OptionalUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, AppError> {
        Ok(Self(current(parts)?.cloned()))
    }
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Response> {
        match current(parts) {
            Err(e) => Err(e.into_response()),
            Ok(Some(u)) => Ok(Self(u.clone())),
            Ok(None) => Err(Redirect::to("/login").into_response()),
        }
    }
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, AppError> {
        match current(parts)? {
            Some(u) if u.is_admin => Ok(Self(u.clone())),
            _ => Err(AppError::NotFound("admin extractor".into())),
        }
    }
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for FerrisUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, AppError> {
        match current(parts)? {
            Some(u) if u.is_ferris => Ok(Self(u.clone())),
            _ => Err(AppError::NotFound("ferris extractor".into())),
        }
    }
}
