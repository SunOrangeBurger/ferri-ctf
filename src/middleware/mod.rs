pub mod admin_guard;
pub mod auth;
pub mod csrf_guard;
pub mod response;

use std::sync::Arc;

use axum::{
    middleware::{from_fn, from_fn_with_state},
    Router,
};
use tower::ServiceBuilder;

use crate::{errors::AppError, state::AppState};

/// The ONE response for "nothing here". Admin routes and real misses must be
/// byte-identical (spec 9.3), so axum's default empty 404 must never be used.
pub async fn fallback() -> AppError {
    AppError::NotFound("no such route".into())
}

/// Install the security stack. Order, outermost first:
///   1. load_session: cookie -> user
///   2. csrf_guard:   token + origin on every unsafe method, for ALL paths
///   3. admin_gate:   404 for non-privileged callers on /admin/*
/// CSRF runs before the admin gate on purpose: an unauthenticated POST gets the same 403
/// for /admin/x as for /anything-else, so the 404 cannot be used to probe for admin paths.
pub fn secure(router: Router<Arc<AppState>>, state: Arc<AppState>) -> Router {
    router
        .fallback(fallback) // must be set BEFORE layer() so the layers wrap it too
        .layer(
            ServiceBuilder::new()
                .layer(from_fn_with_state(state.clone(), auth::load_session))
                .layer(from_fn_with_state(state.clone(), csrf_guard::csrf_guard))
                .layer(from_fn(admin_guard::admin_gate)),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        middleware::{
            auth::{AdminUser, AuthUser},
            csrf_guard::CsrfCtx,
        },
        session,
        state::test_support,
    };
    use axum::{
        body::Body,
        extract::State,
        http::{header, HeaderMap, Request, StatusCode},
        routing::{get, post},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    type Resp = (StatusCode, HeaderMap, String);

    async fn app() -> (Router, Arc<AppState>, tempfile::TempDir) {
        let (state, dir) = test_support::state().await;
        let routes = Router::new()
            .route("/admin/dashboard", get(|| async { "dash" }))
            .route("/admin/rounds", get(|| async { "rounds" }))
            .route("/admin/rounds/start", post(|| async { "started" }))
            .route("/admin/login", get(|| async { "login" }).post(|| async { "logged" }))
            .route("/me", get(|AuthUser(u): AuthUser| async move { u.username }))
            .route("/guarded", get(|AdminUser(u): AdminUser| async move { u.username }))
            .route(
                "/form",
                get(|State(s): State<Arc<AppState>>, ctx: CsrfCtx| async move { ctx.token(&s.keys) }),
            )
            .route("/echo", post(|| async { "posted" }));
        (secure(routes, state.clone()), state, dir)
    }

    fn rq(method: &str, uri: &str, cookie: Option<&str>, extra: &[(&str, &str)], body: &str) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri).header(header::HOST, "localhost");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    }

    fn form_post(uri: &str, cookie: &str, token: &str) -> Request<Body> {
        rq(
            "POST",
            uri,
            Some(cookie),
            &[("content-type", "application/x-www-form-urlencoded")],
            &format!("csrf_token={token}&x=1"),
        )
    }

    async fn send(app: &Router, req: Request<Body>) -> Resp {
        let resp = app.clone().oneshot(req).await.unwrap();
        let (parts, body) = resp.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn shape(r: &Resp) -> (StatusCode, Option<String>, String) {
        let ct = r.1.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned);
        (r.0, ct, r.2.clone())
    }

    fn set_cookie(h: &HeaderMap, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        h.get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|s| s.strip_prefix(prefix.as_str()))
            .map(|rest| rest.split(';').next().unwrap().to_owned())
    }

    /// Returns (cookie header value, csrf token) for an identity.
    async fn ctx(app: &Router, cookie: Option<&str>) -> (String, String) {
        let (st, h, token) = send(app, rq("GET", "/form", cookie, &[], "")).await;
        assert_eq!(st, StatusCode::OK);
        let ck = match cookie {
            Some(c) => c.to_owned(),
            None => format!("ferris_pre={}", set_cookie(&h, "ferris_pre").expect("pre cookie issued")),
        };
        (ck, token)
    }

    async fn login_as(state: &Arc<AppState>, id: &str, admin: bool, ferris: bool) -> String {
        sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
             VALUES (?, ?, 'N', 'x', ?, ?)",
        )
        .bind(id)
        .bind(format!("user-{id}"))
        .bind(admin as i64)
        .bind(ferris as i64)
        .execute(&state.db)
        .await
        .unwrap();
        let c = session::create(&state.db, &state.keys, 24, id).await.unwrap();
        format!("{}={}", session::COOKIE_NAME, c)
    }

    #[tokio::test]
    async fn admin_routes_are_indistinguishable_from_missing_routes() {
        let (app, st, _d) = app().await;
        let player = login_as(&st, "p", false, false).await;
        let admin = login_as(&st, "a", true, false).await;
        let ids: [Option<&str>; 3] = [None, Some(player.as_str()), Some(admin.as_str())];

        let missing_get = shape(&send(&app, rq("GET", "/definitely-missing", None, &[], "")).await);
        assert_eq!(missing_get.0, StatusCode::NOT_FOUND);
        let missing_post_no_token = shape(&send(&app, rq("POST", "/definitely-missing", None, &[], "")).await);
        assert_eq!(missing_post_no_token.0, StatusCode::FORBIDDEN);

        for id in ids {
            for path in ["/admin/rounds", "/admin/nope", "/admin"] {
                let r = send(&app, rq("GET", path, id, &[], "")).await;
                assert_eq!(shape(&r), missing_get, "GET {path} as {id:?}");
            }
            let r = send(&app, rq("POST", "/admin/rounds/start", id, &[], "")).await;
            assert_eq!(shape(&r), missing_post_no_token, "POST without token as {id:?}");

            let (ck, token) = ctx(&app, id).await;
            let missing = shape(&send(&app, form_post("/definitely-missing", &ck, &token)).await);
            assert_eq!(missing.0, StatusCode::NOT_FOUND);
            let r = send(&app, form_post("/admin/rounds/start", &ck, &token)).await;
            assert_eq!(shape(&r), missing, "POST with token as {id:?}");
        }

        for id in [None, Some(player.as_str())] {
            let r = send(&app, rq("GET", "/admin/dashboard", id, &[], "")).await;
            assert_eq!(shape(&r), missing_get, "admin dashboard as {id:?}");
        }
    }

    #[tokio::test]
    async fn admin_and_ferris_levels() {
        let (app, st, _d) = app().await;
        let admin = login_as(&st, "a", true, false).await;
        let ferris = login_as(&st, "f", true, true).await;
        let missing = shape(&send(&app, rq("GET", "/definitely-missing", None, &[], "")).await);

        let r = send(&app, rq("GET", "/admin/dashboard", Some(admin.as_str()), &[], "")).await;
        assert_eq!((r.0, r.2.as_str()), (StatusCode::OK, "dash"));
        let r = send(&app, rq("GET", "/admin/rounds", Some(admin.as_str()), &[], "")).await;
        assert_eq!(shape(&r), missing, "admin must not reach Ferris routes");

        let r = send(&app, rq("GET", "/admin/rounds", Some(ferris.as_str()), &[], "")).await;
        assert_eq!((r.0, r.2.as_str()), (StatusCode::OK, "rounds"));
        let r = send(&app, rq("GET", "/admin/dashboard", Some(ferris.as_str()), &[], "")).await;
        assert_eq!(r.0, StatusCode::OK);
        let r = send(&app, rq("GET", "/admin/nope", Some(ferris.as_str()), &[], "")).await;
        assert_eq!(shape(&r), missing);

        let (ck, token) = ctx(&app, Some(ferris.as_str())).await;
        let r = send(&app, form_post("/admin/rounds/start", &ck, &token)).await;
        assert_eq!((r.0, r.2.as_str()), (StatusCode::OK, "started"));
    }

    #[tokio::test]
    async fn ferris_flag_without_admin_is_not_ferris() {
        let (app, st, _d) = app().await;
        let odd = login_as(&st, "x", false, true).await;
        let missing = shape(&send(&app, rq("GET", "/definitely-missing", None, &[], "")).await);
        for path in ["/admin/rounds", "/admin/dashboard"] {
            let r = send(&app, rq("GET", path, Some(odd.as_str()), &[], "")).await;
            assert_eq!(shape(&r), missing, "{path}");
        }
    }

    #[tokio::test]
    async fn admin_login_is_public_but_csrf_protected() {
        let (app, _st, _d) = app().await;
        let r = send(&app, rq("GET", "/admin/login", None, &[], "")).await;
        assert_eq!((r.0, r.2.as_str()), (StatusCode::OK, "login"));
        let r = send(&app, rq("POST", "/admin/login", None, &[], "")).await;
        assert_eq!(r.0, StatusCode::FORBIDDEN);
        let (ck, token) = ctx(&app, None).await;
        let r = send(&app, form_post("/admin/login", &ck, &token)).await;
        assert_eq!((r.0, r.2.as_str()), (StatusCode::OK, "logged"));
    }

    #[tokio::test]
    async fn csrf_roundtrip_and_rejections() {
        let (app, _st, _d) = app().await;
        let (ck, token) = ctx(&app, None).await;
        assert_eq!(send(&app, form_post("/echo", &ck, &token)).await.0, StatusCode::OK);

        let mut bad = token.clone();
        bad.pop();
        bad.push(if token.ends_with('0') { '1' } else { '0' });
        assert_eq!(send(&app, form_post("/echo", &ck, &bad)).await.0, StatusCode::FORBIDDEN);

        let (ck2, token2) = ctx(&app, None).await;
        assert_ne!(ck, ck2);
        assert_eq!(send(&app, form_post("/echo", &ck, &token2)).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, form_post("/echo", &ck2, &token)).await.0, StatusCode::FORBIDDEN);

        let r = send(
            &app,
            rq(
                "POST",
                "/echo",
                None,
                &[("content-type", "application/x-www-form-urlencoded")],
                &format!("csrf_token={token}"),
            ),
        )
        .await;
        assert_eq!(r.0, StatusCode::FORBIDDEN, "no pre cookie at all");

        assert_eq!(
            send(&app, form_post("/echo", "ferris_pre=short", &token)).await.0,
            StatusCode::FORBIDDEN,
            "malformed pre cookie"
        );
        assert_eq!(
            send(&app, rq("POST", "/echo", Some(ck.as_str()), &[], "")).await.0,
            StatusCode::FORBIDDEN,
            "missing token"
        );
    }

    #[tokio::test]
    async fn csrf_token_transports_origin_and_size_limits() {
        let (app, _st, _d) = app().await;
        let (ck, token) = ctx(&app, None).await;

        let r = send(&app, rq("POST", "/echo", Some(ck.as_str()), &[("x-csrf-token", token.as_str())], "")).await;
        assert_eq!(r.0, StatusCode::OK, "header token");

        let uri = format!("/echo?csrf_token={token}");
        let r = send(&app, rq("POST", &uri, Some(ck.as_str()), &[], "")).await;
        assert_eq!(r.0, StatusCode::OK, "query token (multipart forms)");

        let with_origin = |o: &str| {
            rq(
                "POST",
                "/echo",
                Some(ck.as_str()),
                &[("x-csrf-token", token.as_str()), ("origin", o)],
                "",
            )
        };
        assert_eq!(send(&app, with_origin("http://localhost")).await.0, StatusCode::OK);
        for bad in ["http://evil.example", "null", "http://localhost.evil.example"] {
            assert_eq!(send(&app, with_origin(bad)).await.0, StatusCode::FORBIDDEN, "origin {bad}");
        }

        let big = "a=1&".repeat(100_000);
        let r = send(
            &app,
            rq(
                "POST",
                "/echo",
                Some(ck.as_str()),
                &[("content-type", "application/x-www-form-urlencoded")],
                &big,
            ),
        )
        .await;
        assert_eq!(r.0, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn csrf_tokens_are_bound_to_the_session() {
        let (app, st, _d) = app().await;
        let c1 = login_as(&st, "u1", false, false).await;
        let c2 = login_as(&st, "u2", false, false).await;

        let (ck1, t1) = ctx(&app, Some(c1.as_str())).await;
        assert_eq!(ck1, c1);
        assert_eq!(send(&app, form_post("/echo", &c1, &t1)).await.0, StatusCode::OK);

        let (_, t2) = ctx(&app, Some(c2.as_str())).await;
        assert_eq!(send(&app, form_post("/echo", &c1, &t2)).await.0, StatusCode::FORBIDDEN);

        let (_, anon) = ctx(&app, None).await;
        assert_eq!(send(&app, form_post("/echo", &c1, &anon)).await.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn auth_extractors() {
        let (app, st, _d) = app().await;
        let missing = shape(&send(&app, rq("GET", "/definitely-missing", None, &[], "")).await);

        let r = send(&app, rq("GET", "/me", None, &[], "")).await;
        assert_eq!(r.0, StatusCode::SEE_OTHER);
        assert_eq!(r.1.get(header::LOCATION).unwrap().to_str().unwrap(), "/login");

        let c = login_as(&st, "u1", false, false).await;
        let r = send(&app, rq("GET", "/me", Some(c.as_str()), &[], "")).await;
        assert_eq!((r.0, r.2.as_str()), (StatusCode::OK, "user-u1"));

        let r = send(&app, rq("GET", "/guarded", Some(c.as_str()), &[], "")).await;
        assert_eq!(shape(&r), missing, "AdminUser extractor must 404 for players");

        for junk in ["ferris_session=garbage", "ferris_session=", "ferris_session=a.b"] {
            let r = send(&app, rq("GET", "/me", Some(junk), &[], "")).await;
            assert_eq!(r.0, StatusCode::SEE_OTHER, "{junk}");
        }

        sqlx::query("UPDATE users SET is_banned = 1 WHERE id = 'u1'")
            .execute(&st.db)
            .await
            .unwrap();
        let r = send(&app, rq("GET", "/me", Some(c.as_str()), &[], "")).await;
        assert_eq!(r.0, StatusCode::SEE_OTHER, "banned user is anonymous immediately");
    }

    #[tokio::test]
    async fn pre_cookie_is_only_issued_when_a_token_is_requested() {
        let (app, _st, _d) = app().await;
        let r = send(&app, rq("GET", "/definitely-missing", None, &[], "")).await;
        assert!(set_cookie(&r.1, "ferris_pre").is_none());

        let r = send(&app, rq("GET", "/form", None, &[], "")).await;
        let v = set_cookie(&r.1, "ferris_pre").expect("cookie issued");
        assert_eq!(v.len(), 32);
        let raw = r
            .1
            .get_all(header::SET_COOKIE)
            .iter()
            .next()
            .unwrap()
            .to_str()
            .unwrap()
            .to_lowercase();
        assert!(raw.contains("httponly") && raw.contains("samesite=strict"), "{raw}");

        let c = format!("ferris_pre={v}");
        let r = send(&app, rq("GET", "/form", Some(c.as_str()), &[], "")).await;
        assert!(set_cookie(&r.1, "ferris_pre").is_none(), "no reissue when cookie present");
    }
}
