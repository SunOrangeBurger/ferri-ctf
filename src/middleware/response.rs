use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};
use minijinja::context;

use crate::{errors::PublicError, state::AppState};

/// Spec 9.7, plus form-action, frame-ancestors and base-uri as extra hardening.
const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
                   form-action 'self'; frame-ancestors 'none'; base-uri 'self'";
const HSTS: &str = "max-age=31536000; includeSubDomains";

/// Outermost layer: stamps every response, including errors from inner layers.
/// `same-origin` (not the spec's `no-referrer`) keeps the Origin header meaningful for
/// the CSRF origin check; cross-origin requests still leak no referrer.
pub async fn security_headers(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if state.cfg.cookie_secure {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static(HSTS));
    }
    // Dynamic pages are never cached; handlers that set their own policy win.
    h.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    resp
}

/// Swaps plain-text error bodies for the themed page. The page is rendered WITHOUT any
/// user context, so a 404 for an admin route is byte-identical to a 404 for a typo, for
/// every kind of caller (spec 9.3). JSON routes under /api/ keep their own bodies.
pub async fn error_pages(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let resp = next.run(req).await;
    if is_api {
        return resp;
    }
    let Some(err) = resp.extensions().get::<PublicError>().cloned() else {
        return resp;
    };
    match state.templates.render(
        "errors/error.html",
        context! { status => err.status.as_u16(), message => err.message },
    ) {
        Ok(Html(body)) => (err.status, Html(body)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "error page failed to render; sending plain response");
            resp
        }
    }
}
