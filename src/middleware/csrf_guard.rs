use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use axum::{
    async_trait,
    body::{to_bytes, Body},
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts, HeaderMap, HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::{
    cookie::{Cookie, SameSite},
    CookieJar,
};
use rand::{rngs::OsRng, RngCore};

use crate::{csrf, errors::AppError, keys::Keys, middleware::auth::CurrentUser, state::AppState};

/// Binds CSRF tokens for anonymous visitors (login/register have no session yet).
pub const PRE_COOKIE: &str = "ferris_pre";
/// Largest urlencoded form we will buffer to look for the token.
const MAX_FORM_BYTES: usize = 256 * 1024;

/// Handed to handlers so templates can embed a token. The pre-session cookie is only
/// issued if a handler actually asks for a token.
#[derive(Clone)]
pub struct CsrfCtx {
    binding: String,
    used: Arc<AtomicBool>,
}

impl CsrfCtx {
    pub fn token(&self, keys: &Keys) -> String {
        self.used.store(true, Ordering::Relaxed);
        csrf::issue(keys, &self.binding)
    }
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for CsrfCtx {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, AppError> {
        parts
            .extensions
            .get::<CsrfCtx>()
            .cloned()
            .ok_or_else(|| AppError::internal("csrf middleware not installed"))
    }
}

fn valid_pre(v: &str) -> bool {
    v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit())
}

fn new_pre_id() -> String {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

pub async fn csrf_guard(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let session_binding = req
        .extensions()
        .get::<CurrentUser>()
        .and_then(|c| c.0.as_ref())
        .map(|u| u.session_id.clone());

    let (binding, fresh_pre) = match session_binding {
        Some(id) => (id, false),
        None => match jar.get(PRE_COOKIE).map(|c| c.value().to_owned()).filter(|v| valid_pre(v)) {
            Some(v) => (v, false),
            None => (new_pre_id(), true),
        },
    };

    // Everything except GET/HEAD/OPTIONS needs a token (fail closed: includes TRACE etc).
    if !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        if let Err(e) = check_origin(req.headers()) {
            return e.into_response();
        }

        let mut token = token_from_header(req.headers()).or_else(|| req.uri().query().and_then(find_field));
        if token.is_none() && is_urlencoded(req.headers()) {
            let (parts, body) = req.into_parts();
            let bytes = match to_bytes(body, MAX_FORM_BYTES).await {
                Ok(b) => b,
                Err(_) => return AppError::PayloadTooLarge.into_response(),
            };
            token = std::str::from_utf8(&bytes).ok().and_then(find_field);
            req = Request::from_parts(parts, Body::from(bytes));
        }

        let ok = token
            .as_deref()
            .is_some_and(|t| csrf::verify(&state.keys, &binding, t));
        if !ok {
            return AppError::Forbidden("Invalid or missing CSRF token".into()).into_response();
        }
    }

    let used = Arc::new(AtomicBool::new(false));
    req.extensions_mut().insert(CsrfCtx { binding: binding.clone(), used: used.clone() });
    let mut resp = next.run(req).await;

    if fresh_pre && used.load(Ordering::Relaxed) {
        let c = Cookie::build((PRE_COOKIE, binding))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Strict)
            .secure(state.cfg.cookie_secure)
            .build();
        if let Ok(v) = HeaderValue::from_str(&c.to_string()) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    resp
}

fn token_from_header(h: &HeaderMap) -> Option<String> {
    h.get(csrf::HEADER_NAME)?.to_str().ok().map(str::to_owned)
}

/// Looks for `csrf_token=` in a urlencoded string. Tokens are hex plus '.', so no
/// percent-decoding is needed; anything odd simply fails verification.
fn find_field(s: &str) -> Option<String> {
    s.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == csrf::FORM_FIELD).then(|| v.to_owned())
    })
}

fn is_urlencoded(h: &HeaderMap) -> bool {
    h.get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/x-www-form-urlencoded")
        })
}

fn origin_authority(v: &str) -> Option<&str> {
    let rest = v.strip_prefix("http://").or_else(|| v.strip_prefix("https://"))?;
    rest.split(['/', '?', '#']).next()
}

/// Second CSRF layer: if the browser says where the request came from, it must be us.
/// No Origin and no Referer (non-browser clients) is allowed; the token is still required.
fn check_origin(headers: &HeaderMap) -> Result<(), AppError> {
    let claimed = if let Some(v) = headers.get(header::ORIGIN) {
        v.to_str().unwrap_or("null")
    } else if let Some(v) = headers.get(header::REFERER) {
        v.to_str().unwrap_or("null")
    } else {
        return Ok(());
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    match (origin_authority(claimed), host) {
        (Some(a), Some(h)) if a.eq_ignore_ascii_case(h) => Ok(()),
        _ => Err(AppError::Forbidden("Cross-origin request blocked".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        m
    }

    #[test]
    fn origin_rules() {
        assert!(check_origin(&h(&[("host", "a:8080")])).is_ok());
        assert!(check_origin(&h(&[("host", "a:8080"), ("origin", "http://a:8080")])).is_ok());
        assert!(check_origin(&h(&[("host", "A:8080"), ("origin", "http://a:8080")])).is_ok());
        assert!(check_origin(&h(&[("host", "a:8080"), ("origin", "http://evil")])).is_err());
        assert!(check_origin(&h(&[("host", "a:8080"), ("origin", "null")])).is_err());
        assert!(check_origin(&h(&[("host", "a:8080"), ("origin", "http://a:8080.evil.com")])).is_err());
        assert!(check_origin(&h(&[("host", "a:8080"), ("referer", "http://a:8080/login")])).is_ok());
        assert!(check_origin(&h(&[("host", "a:8080"), ("referer", "http://evil/a:8080")])).is_err());
        assert!(check_origin(&h(&[("origin", "http://a:8080")])).is_err());
    }

    #[test]
    fn field_parsing() {
        assert_eq!(find_field("x=1&csrf_token=ab.cd&y=2").as_deref(), Some("ab.cd"));
        assert_eq!(find_field("x=1"), None);
    }
}
