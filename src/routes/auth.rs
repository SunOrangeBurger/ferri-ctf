use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::CookieJar;
use minijinja::context;
use serde::Deserialize;

use crate::{
    errors::{AppError, AppResult},
    middleware::{
        auth::{AuthUser, OptionalUser},
        csrf_guard::CsrfCtx,
    },
    ratelimit::{self, ClientIp},
    services::auth::{self as auth_svc, LoginResult},
    session,
    state::AppState,
};

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct RegisterForm {
    username: String,
    display_name: String,
    password: String,
    password_confirm: String,
}

fn render_login(
    state: &AppState,
    ctx: &CsrfCtx,
    status: StatusCode,
    username: &str,
    error: Option<&str>,
) -> AppResult<Response> {
    let page = state.templates.render(
        "auth/login.html",
        context! { csrf_token => ctx.token(&state.keys), username => username, error => error },
    )?;
    Ok((status, page).into_response())
}

fn render_register(
    state: &AppState,
    ctx: &CsrfCtx,
    status: StatusCode,
    username: &str,
    display_name: &str,
    error: Option<&str>,
) -> AppResult<Response> {
    let page = state.templates.render(
        "auth/register.html",
        context! {
            csrf_token => ctx.token(&state.keys),
            username => username,
            display_name => display_name,
            error => error,
        },
    )?;
    Ok((status, page).into_response())
}

fn render_admin_login(
    state: &AppState,
    ctx: &CsrfCtx,
    status: StatusCode,
    error: Option<&str>,
) -> AppResult<Response> {
    let page = state.templates.render(
        "admin/login.html",
        context! { csrf_token => ctx.token(&state.keys), error => error },
    )?;
    Ok((status, page).into_response())
}

// ---------- /login ----------

pub async fn login_page(
    State(state): State<Arc<AppState>>,
    OptionalUser(user): OptionalUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    if user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    render_login(&state, &ctx, StatusCode::OK, "", None)
}

pub async fn login_submit(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    OptionalUser(current): OptionalUser,
    ctx: CsrfCtx,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> AppResult<Response> {
    if !state.limiter.check("login", &ip, ratelimit::LOGIN) {
        return Err(AppError::TooManyRequests);
    }
    let previous = current.as_ref().map(|u| u.session_id.as_str());
    match auth_svc::login(&state, &form.username, &form.password, false, previous).await? {
        LoginResult::Success { cookie_value, .. } => {
            let jar = jar.add(session::cookie(cookie_value, state.cfg.cookie_secure));
            Ok((jar, Redirect::to("/")).into_response())
        }
        LoginResult::InvalidCredentials => render_login(
            &state,
            &ctx,
            StatusCode::UNAUTHORIZED,
            &form.username,
            Some("Invalid college ID or password"),
        ),
        LoginResult::Banned => render_login(
            &state,
            &ctx,
            StatusCode::FORBIDDEN,
            &form.username,
            Some("This account is suspended. Please contact an organizer."),
        ),
    }
}

// ---------- /register ----------

pub async fn register_page(
    State(state): State<Arc<AppState>>,
    OptionalUser(user): OptionalUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    if user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    render_register(&state, &ctx, StatusCode::OK, "", "", None)
}

pub async fn register_submit(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    OptionalUser(current): OptionalUser,
    ctx: CsrfCtx,
    jar: CookieJar,
    Form(form): Form<RegisterForm>,
) -> AppResult<Response> {
    if current.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    if !state.limiter.check("register", &ip, ratelimit::REGISTER) {
        return Err(AppError::TooManyRequests);
    }
    if form.password != form.password_confirm {
        return render_register(
            &state,
            &ctx,
            StatusCode::BAD_REQUEST,
            &form.username,
            &form.display_name,
            Some("Passwords do not match"),
        );
    }

    match auth_svc::register(&state, &form.username, &form.display_name, &form.password).await {
        Ok(user_id) => {
            // Sign the new user in directly; a second Argon2 verification would be wasted work.
            let cookie_value =
                session::create(&state.db, &state.keys, state.cfg.session_ttl_hours, &user_id).await?;
            let jar = jar.add(session::cookie(cookie_value, state.cfg.cookie_secure));
            Ok((jar, Redirect::to("/")).into_response())
        }
        Err(AppError::BadRequest(m)) => render_register(
            &state, &ctx, StatusCode::BAD_REQUEST, &form.username, &form.display_name, Some(&m),
        ),
        Err(AppError::Conflict(m)) => render_register(
            &state, &ctx, StatusCode::CONFLICT, &form.username, &form.display_name, Some(&m),
        ),
        Err(e) => Err(e),
    }
}

// ---------- /logout ----------

pub async fn logout(
    State(state): State<Arc<AppState>>,
    AuthUser(user): AuthUser,
    jar: CookieJar,
) -> AppResult<Response> {
    session::destroy(&state.db, &user.session_id).await?;
    Ok((jar.remove(session::removal_cookie()), Redirect::to("/login")).into_response())
}

// ---------- /admin/login (the one public admin route, spec 9.3) ----------

pub async fn admin_login_page(
    State(state): State<Arc<AppState>>,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    render_admin_login(&state, &ctx, StatusCode::OK, None)
}

pub async fn admin_login_submit(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    OptionalUser(current): OptionalUser,
    ctx: CsrfCtx,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> AppResult<Response> {
    if !state.limiter.check("admin_login", &ip, ratelimit::ADMIN_LOGIN) {
        return Err(AppError::TooManyRequests);
    }
    let previous = current.as_ref().map(|u| u.session_id.as_str());
    match auth_svc::login(&state, &form.username, &form.password, true, previous).await? {
        LoginResult::Success { cookie_value, .. } => {
            let jar = jar.add(session::cookie(cookie_value, state.cfg.cookie_secure));
            Ok((jar, Redirect::to("/admin/dashboard")).into_response())
        }
        // Unknown user, wrong password, not an admin, banned: one message for all of them.
        LoginResult::InvalidCredentials | LoginResult::Banned => {
            render_admin_login(&state, &ctx, StatusCode::UNAUTHORIZED, Some("Invalid credentials"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app, state::test_support};
    use axum::{
        body::Body,
        http::{header, HeaderMap, Request},
        Router,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const PW: &str = "correct-horse-battery";
    type Resp = (StatusCode, HeaderMap, String);

    async fn setup() -> (Router, Arc<AppState>, tempfile::TempDir) {
        let (state, dir) = test_support::state().await;
        (app::build(state.clone()), state, dir)
    }

    fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri(uri).header(header::HOST, "localhost");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        b.body(Body::empty()).unwrap()
    }

    fn post(uri: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    }

    async fn send(app: &Router, req: Request<Body>) -> Resp {
        let resp = app.clone().oneshot(req).await.unwrap();
        let (parts, body) = resp.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn set_cookie(h: &HeaderMap, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        h.get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|s| s.strip_prefix(prefix.as_str()))
            .map(|rest| rest.split(';').next().unwrap().to_owned())
    }

    fn find_token(html: &str) -> String {
        let marker = "name=\"csrf_token\" value=\"";
        let start = html.find(marker).expect("csrf field in page") + marker.len();
        html[start..].split('"').next().unwrap().to_owned()
    }

    fn location(h: &HeaderMap) -> &str {
        h.get(header::LOCATION).unwrap().to_str().unwrap()
    }

    fn login_body(user: &str, pw: &str, token: &str) -> String {
        format!("username={user}&password={pw}&csrf_token={token}")
    }

    /// GET a form page anonymously: returns (cookie header, csrf token).
    async fn anon_form(app: &Router, path: &str) -> (String, String) {
        let (s, h, body) = send(app, get(path, None)).await;
        assert_eq!(s, StatusCode::OK, "{path}");
        let pre = set_cookie(&h, "ferris_pre").expect("pre-session cookie issued");
        (format!("ferris_pre={pre}"), find_token(&body))
    }

    #[tokio::test]
    async fn register_login_logout_flow() {
        let (app, _st, _d) = setup().await;
        let (ck, token) = anon_form(&app, "/register").await;
        let body = format!(
            "username=PES2UG24AM001&display_name=Ada+Lovelace&password={PW}&password_confirm={PW}&csrf_token={token}"
        );
        let (s, h, _) = send(&app, post("/register", Some(ck.as_str()), &body)).await;
        assert_eq!(s, StatusCode::SEE_OTHER);
        assert_eq!(location(&h), "/");
        let sess = format!("ferris_session={}", set_cookie(&h, "ferris_session").expect("session cookie"));

        let (s, _, page) = send(&app, get("/", Some(sess.as_str()))).await;
        assert_eq!(s, StatusCode::OK);
        assert!(page.contains("Ada Lovelace") && page.contains("Log out"));
        let (s, h, _) = send(&app, get("/login", Some(sess.as_str()))).await;
        assert_eq!((s, location(&h)), (StatusCode::SEE_OTHER, "/"), "logged-in users skip the login page");

        let logout_token = find_token(&page);
        let (s, h, _) = send(
            &app,
            post("/logout", Some(sess.as_str()), &format!("csrf_token={logout_token}")),
        )
        .await;
        assert_eq!((s, location(&h)), (StatusCode::SEE_OTHER, "/login"));
        let (_, _, page) = send(&app, get("/", Some(sess.as_str()))).await;
        assert!(!page.contains("Ada Lovelace"), "the old cookie must be dead server-side");

        let (ck, token) = anon_form(&app, "/login").await;
        let (s, _, page) = send(
            &app,
            post("/login", Some(ck.as_str()), &login_body("pes2ug24am001", "wrong-password", &token)),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(page.contains("Invalid college ID or password"));
        let (s, h, _) = send(
            &app,
            post("/login", Some(ck.as_str()), &login_body("PES2UG24AM001", PW, &token)),
        )
        .await;
        assert_eq!((s, location(&h)), (StatusCode::SEE_OTHER, "/"));
        assert!(set_cookie(&h, "ferris_session").is_some());
    }

    #[tokio::test]
    async fn register_validation_and_conflicts() {
        let (app, st, _d) = setup().await;
        let (ck, token) = anon_form(&app, "/register").await;
        let form = |u: &str, d: &str, p: &str, c: &str| {
            format!("username={u}&display_name={d}&password={p}&password_confirm={c}&csrf_token={token}")
        };
        let cases = [
            (form("pes2ug24am001", "Ada", PW, "different-pass"), StatusCode::BAD_REQUEST, "do not match"),
            (form("pes2ug24am001", "Ada", "short", "short"), StatusCode::BAD_REQUEST, "at least 8 characters"),
            (form("a+b", "Ada", PW, PW), StatusCode::BAD_REQUEST, "may contain only"),
            (form("pes2ug24am001", "", PW, PW), StatusCode::BAD_REQUEST, "1 to 40 characters"),
        ];
        for (body, want, needle) in cases {
            let (s, _, page) = send(&app, post("/register", Some(ck.as_str()), &body)).await;
            assert_eq!(s, want, "{needle}");
            assert!(page.contains(needle), "{needle}");
        }
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&st.db).await.unwrap();
        assert_eq!(n, 0, "failed registrations create nothing");

        let (s, _, _) = send(&app, post("/register", Some(ck.as_str()), &form("PES2UG24AM001", "Ada", PW, PW))).await;
        assert_eq!(s, StatusCode::SEE_OTHER);
        let (s, _, page) = send(&app, post("/register", Some(ck.as_str()), &form("pes2ug24am001", "Other", PW, PW))).await;
        assert_eq!(s, StatusCode::CONFLICT, "case-variant duplicate");
        assert!(page.contains("already registered"));
    }

    #[tokio::test]
    async fn login_failures_ban_and_rate_limit() {
        let (app, st, _d) = setup().await;
        auth_svc::register(&st, "ada", "Ada", PW).await.unwrap();
        let (ck, token) = anon_form(&app, "/login").await;

        sqlx::query("UPDATE users SET is_banned = 1 WHERE username = 'ada'").execute(&st.db).await.unwrap();
        let (s, _, page) = send(&app, post("/login", Some(ck.as_str()), &login_body("ada", PW, &token))).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert!(page.contains("suspended"));
        sqlx::query("UPDATE users SET is_banned = 0 WHERE username = 'ada'").execute(&st.db).await.unwrap();

        for i in 0..9 {
            let who = if i % 2 == 0 { "ada" } else { "nobody" };
            let req = post("/login", Some(ck.as_str()), &login_body(who, "wrong-password", &token));
            let (s, h, page) = send(&app, req).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "attempt {i}");
            assert!(page.contains("Invalid college ID or password"));
            assert!(set_cookie(&h, "ferris_session").is_none());
        }
        // Ten attempts used. The eleventh is refused even with the correct password.
        let (s, h, page) = send(&app, post("/login", Some(ck.as_str()), &login_body("ada", PW, &token))).await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert!(page.contains("Too many requests"));
        assert!(set_cookie(&h, "ferris_session").is_none());
    }

    #[tokio::test]
    async fn admin_login_flow_and_rate_limit() {
        let (app, st, _d) = setup().await;
        auth_svc::register(&st, "player1", "Player", PW).await.unwrap();
        auth_svc::register(&st, "ferris1", "Boss", PW).await.unwrap();
        sqlx::query("UPDATE users SET is_admin = 1, is_ferris = 1 WHERE username = 'ferris1'")
            .execute(&st.db)
            .await
            .unwrap();

        let (ck, token) = anon_form(&app, "/admin/login").await;
        let attempt = |u: &str, p: &str| post("/admin/login", Some(ck.as_str()), &login_body(u, p, &token));

        let (s, _, page) = send(&app, attempt("player1", PW)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "a player cannot use the admin door");
        assert!(page.contains("Invalid credentials"));
        let (s, _, _) = send(&app, attempt("ferris1", "wrong-password")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, h, _) = send(&app, attempt("ferris1", PW)).await;
        assert_eq!((s, location(&h)), (StatusCode::SEE_OTHER, "/admin/dashboard"));
        let sess = format!("ferris_session={}", set_cookie(&h, "ferris_session").unwrap());

        let (s, _, page) = send(&app, get("/admin/dashboard", Some(sess.as_str()))).await;
        assert_eq!(s, StatusCode::OK);
        assert!(page.contains("Ferris (operator)"));

        let (s, _, _) = send(&app, attempt("ferris1", PW)).await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "fourth attempt in five minutes");
    }

    #[tokio::test]
    async fn security_headers_and_static_files() {
        let (app, _st, _d) = setup().await;
        for path in ["/health", "/nope"] {
            let (_, h, _) = send(&app, get(path, None)).await;
            let v = |n: &str| h.get(n).map(|v| v.to_str().unwrap().to_owned());
            assert!(v("content-security-policy").unwrap().contains("default-src 'self'"), "{path}");
            assert_eq!(v("x-frame-options").as_deref(), Some("DENY"), "{path}");
            assert_eq!(v("referrer-policy").as_deref(), Some("same-origin"), "{path}");
            assert_eq!(v("x-content-type-options").as_deref(), Some("nosniff"), "{path}");
            assert_eq!(v("cache-control").as_deref(), Some("no-store"), "{path}");
            assert!(v("strict-transport-security").is_none(), "no HSTS over plain HTTP");
        }

        let (s, h, body) = send(&app, get("/static/style.css", None)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(h.get("content-type").unwrap().to_str().unwrap().contains("text/css"));
        assert_eq!(h.get("cache-control").unwrap().to_str().unwrap(), "no-cache");
        assert!(body.contains("--accent"));

        for bad in [
            "/static/missing.css",
            "/static/%2e%2e/Cargo.toml",
            "/static/..%2FCargo.toml",
            "/static/a%5Cb",
        ] {
            let (s, _, _) = send(&app, get(bad, None)).await;
            assert_eq!(s, StatusCode::NOT_FOUND, "{bad}");
        }
    }

    #[tokio::test]
    async fn error_pages_are_themed_and_identical() {
        let (app, st, _d) = setup().await;
        let id = auth_svc::register(&st, "player1", "Player", PW).await.unwrap();
        let c = session::create(&st.db, &st.keys, 24, &id).await.unwrap();
        let player = format!("{}={}", session::COOKIE_NAME, c);

        let (s, h, missing) = send(&app, get("/nope", None)).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert!(h.get("content-type").unwrap().to_str().unwrap().starts_with("text/html"));
        assert!(missing.contains("<html") && missing.contains("404") && missing.contains("Not found"));

        for (path, cookie) in [
            ("/admin/rounds", None),
            ("/admin/dashboard", None),
            ("/admin/dashboard", Some(player.as_str())),
            ("/admin/nope", Some(player.as_str())),
            ("/nope", Some(player.as_str())),
        ] {
            let (s, _, body) = send(&app, get(path, cookie)).await;
            assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(body, missing, "{path} must match a plain miss byte for byte");
        }

        let (s, _, body) = send(&app, post("/nope", None, "x=1")).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert!(body.contains("<html") && body.contains("CSRF"));
    }

    #[tokio::test]
    async fn output_is_html_escaped() {
        let (app, _st, _d) = setup().await;
        let (ck, token) = anon_form(&app, "/register").await;
        let evil = "%3Cscript%3Ealert(1)%3C%2Fscript%3E";
        let body = format!(
            "username=xss-tester&display_name={evil}&password={PW}&password_confirm={PW}&csrf_token={token}"
        );
        let (s, h, _) = send(&app, post("/register", Some(ck.as_str()), &body)).await;
        assert_eq!(s, StatusCode::SEE_OTHER);
        let sess = format!("ferris_session={}", set_cookie(&h, "ferris_session").unwrap());
        let (_, _, page) = send(&app, get("/", Some(sess.as_str()))).await;
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("<script>alert"));

        let (ck, token) = anon_form(&app, "/login").await;
        let evil_user = "%22%3E%3Cscript%3Ex%3C%2Fscript%3E";
        let (s, _, page) = send(
            &app,
            post("/login", Some(ck.as_str()), &login_body(evil_user, "wrong-password", &token)),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(!page.contains("<script>x"));
        assert!(page.contains("&lt;script&gt;x"), "the echoed username must be escaped");
    }
}
