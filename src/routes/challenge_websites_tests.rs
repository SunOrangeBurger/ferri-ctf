use crate::{app, csrf, services::dynamic_scoring::tests::fixture, session, state::AppState};
use axum::{
    body::Body,
    http::{header, HeaderMap, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use sqlx::Executor;
use tower::ServiceExt;

async fn identity(state: &AppState, id: &str, admin: bool, ferris: bool) -> (String, String) {
    sqlx::query(
        "INSERT OR IGNORE INTO users (id,username,display_name,password_hash,is_admin,is_ferris)
        VALUES (?,?,?,'unused',?,?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(admin)
    .bind(ferris)
    .execute(&state.db)
    .await
    .unwrap();
    let sealed = session::create(&state.db, &state.keys, 24, id)
        .await
        .unwrap();
    let user = session::authenticate(&state.db, &state.keys, &sealed)
        .await
        .unwrap()
        .unwrap();
    (
        format!("{}={sealed}", session::COOKIE_NAME),
        csrf::issue(&state.keys, &user.session_id),
    )
}

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    credentials: Option<&(String, String)>,
    body: String,
    content_type: &str,
    include_csrf: bool,
) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, content_type);
    if let Some((cookie, token)) = credentials {
        request = request.header(header::COOKIE, cookie);
        if include_csrf {
            request = request.header("x-csrf-token", token);
        }
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&body.collect().await.unwrap().to_bytes()).into_owned(),
    )
}

fn form(url: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("challenge_url", url)
        .finish()
}

async fn set_website(
    app: &Router,
    credentials: Option<&(String, String)>,
    id: &str,
    url: &str,
    csrf: bool,
) -> StatusCode {
    send(
        app,
        "POST",
        &format!("/admin/challenges/{id}/website"),
        credentials,
        form(url),
        "application/x-www-form-urlencoded",
        csrf,
    )
    .await
    .0
}

#[tokio::test]
async fn only_ferris_with_csrf_can_change_a_website() {
    let (state, _directory) = fixture(false, 1).await;
    let router = app::build(state.clone());
    let player = identity(&state, "u0", false, false).await;
    let admin = identity(&state, "admin", true, false).await;
    let operator = identity(&state, "operator", true, true).await;
    for credentials in [None, Some(&player), Some(&admin)] {
        let status = set_website(
            &router,
            credentials,
            "easy",
            "https://q01.example.org/",
            true,
        )
        .await;
        assert!(matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
        ));
    }
    assert_eq!(
        set_website(
            &router,
            Some(&operator),
            "easy",
            "https://q01.example.org/",
            false
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let saved: Option<String> =
        sqlx::query_scalar("SELECT challenge_url FROM challenges WHERE id='easy'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(saved.is_none());
    let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(audit, 0);
}

#[tokio::test]
async fn operator_can_set_replace_and_clear_only_the_website() {
    let (state, _directory) = fixture(false, 1).await;
    let router = app::build(state.clone());
    let operator = identity(&state, "operator", true, true).await;
    let before: (String, String, i64, i64) = sqlx::query_as(
        "SELECT flag_hmac,flag_salt,points,is_active FROM challenges WHERE id='easy'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    for url in ["https://q01.example.org/", "http://192.168.1.10:9101/", ""] {
        assert_eq!(
            set_website(&router, Some(&operator), "easy", url, true).await,
            StatusCode::SEE_OTHER
        );
        let saved: Option<String> =
            sqlx::query_scalar("SELECT challenge_url FROM challenges WHERE id='easy'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(
            saved.as_deref(),
            if url.is_empty() { None } else { Some(url) }
        );
    }
    assert_eq!(
        set_website(
            &router,
            Some(&operator),
            "easy",
            "javascript:alert(1)",
            true
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        set_website(
            &router,
            Some(&operator),
            "missing",
            "https://q01.example.org/",
            true
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let after: (String, String, i64, i64) = sqlx::query_as(
        "SELECT flag_hmac,flag_salt,points,is_active FROM challenges WHERE id='easy'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(before, after);
    let audit: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action='set_challenge_website'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(audit, 3);
    let pool: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM team_challenge_pool")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(pool, 3);
}

fn multipart(title: &str, website: Option<&str>) -> String {
    let mut fields = vec![
        ("title", title),
        ("description", "Open the puzzle and submit its flag here."),
        ("category", "Web"),
        ("difficulty", "easy"),
        ("points", "100"),
        ("flag", "ferris{correct}"),
    ];
    if let Some(url) = website {
        fields.push(("challenge_url", url));
    }
    let mut body = String::new();
    for (name, value) in fields {
        body.push_str(&format!(
            "--website-test\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str("--website-test--\r\n");
    body
}

#[tokio::test]
async fn creation_accepts_link_only_and_legacy_forms_and_rejects_unsafe_links() {
    let (state, _directory) = fixture(false, 0).await;
    let router = app::build(state.clone());
    let operator = identity(&state, "operator", true, true).await;
    for (title, website, status) in [
        (
            "Hosted puzzle",
            Some("https://q01.example.org"),
            StatusCode::SEE_OTHER,
        ),
        ("Legacy puzzle", None, StatusCode::SEE_OTHER),
        (
            "Bad puzzle",
            Some("data:text/html,attack"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let result = send(
            &router,
            "POST",
            "/admin/challenges/new",
            Some(&operator),
            multipart(title, website),
            "multipart/form-data; boundary=website-test",
            true,
        )
        .await;
        assert_eq!(result.0, status, "{title}");
    }
    let hosted: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT challenge_url,file_path FROM challenges WHERE title='Hosted puzzle'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(hosted, (Some("https://q01.example.org/".into()), None));
    let legacy: Option<String> =
        sqlx::query_scalar("SELECT challenge_url FROM challenges WHERE title='Legacy puzzle'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(legacy.is_none());
    let bad: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges WHERE title='Bad puzzle'")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(bad, 0);
    let page = send(
        &router,
        "GET",
        "/admin/challenges",
        Some(&operator),
        String::new(),
        "text/plain",
        false,
    )
    .await;
    assert_eq!(page.0, StatusCode::OK);
    assert!(page.2.contains("https://q01.example.org/"));
    assert!(page.2.contains("/website\""));
}

#[tokio::test]
async fn launch_links_follow_existing_pool_round_team_and_visibility_rules() {
    let (state, _directory) = fixture(false, 1).await;
    sqlx::query("UPDATE challenges SET challenge_url='https://q01.example.org/play?view=evidence&next=finish' WHERE id IN ('easy','hidden')")
        .execute(&state.db).await.unwrap();
    let router = app::build(state.clone());
    let player = identity(&state, "u0", false, false).await;
    let page = send(
        &router,
        "GET",
        "/challenges/easy",
        Some(&player),
        String::new(),
        "text/plain",
        false,
    )
    .await;
    assert_eq!(page.0, StatusCode::OK);
    assert!(page.2.contains("Open challenge website"));
    assert!(page.2.contains("q01.example.org"));
    assert!(page.2.contains("view=evidence&amp;next=finish"));
    assert!(page
        .2
        .contains("target=\"_blank\" rel=\"noopener noreferrer\""));
    let legacy = send(
        &router,
        "GET",
        "/challenges/medium",
        Some(&player),
        String::new(),
        "text/plain",
        false,
    )
    .await;
    assert_eq!(legacy.0, StatusCode::OK);
    assert!(!legacy.2.contains("Open challenge website"));
    let hidden = send(
        &router,
        "GET",
        "/challenges/hidden",
        Some(&player),
        String::new(),
        "text/plain",
        false,
    )
    .await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert!(!hidden.2.contains("q01.example.org"));
    let guest = send(
        &router,
        "GET",
        "/challenges/easy",
        None,
        String::new(),
        "text/plain",
        false,
    )
    .await;
    assert_ne!(guest.0, StatusCode::OK);
    assert!(!guest.2.contains("q01.example.org"));

    // Opening a hosted puzzle leaves the ordinary FerrisCTF flag submission intact.
    let submitted = send(
        &router,
        "POST",
        "/challenges/easy/submit",
        Some(&player),
        "flag=ferris%7Bcorrect%7D".into(),
        "application/x-www-form-urlencoded",
        true,
    )
    .await;
    assert_eq!(submitted.0, StatusCode::SEE_OTHER);
    assert!(submitted.1[header::LOCATION]
        .to_str()
        .unwrap()
        .contains("status=correct"));

    for (block, restore) in [
        (
            "UPDATE teams SET is_locked=0 WHERE id='t0'",
            "UPDATE teams SET is_locked=1 WHERE id='t0'",
        ),
        (
            "UPDATE teams SET disbanded=1 WHERE id='t0'",
            "UPDATE teams SET disbanded=0 WHERE id='t0'",
        ),
        (
            "UPDATE challenges SET is_active=0 WHERE id='easy'",
            "UPDATE challenges SET is_active=1 WHERE id='easy'",
        ),
        (
            "UPDATE rounds SET ended_at=CURRENT_TIMESTAMP",
            "UPDATE rounds SET ended_at=NULL",
        ),
    ] {
        sqlx::query(block).execute(&state.db).await.unwrap();
        let denied = send(
            &router,
            "GET",
            "/challenges/easy",
            Some(&player),
            String::new(),
            "text/plain",
            false,
        )
        .await;
        assert_eq!(denied.0, StatusCode::NOT_FOUND, "{block}");
        assert!(!denied.2.contains("q01.example.org"));
        sqlx::query(restore).execute(&state.db).await.unwrap();
    }
}

#[tokio::test]
async fn migration_preserves_existing_challenge_and_attachment_data() {
    let database = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    database
        .execute(include_str!("../../migrations/0001_init.sql"))
        .await
        .unwrap();
    sqlx::query("INSERT INTO challenges(id,title,description,category,difficulty,points,flag_hmac,flag_salt,file_path,file_name)
        VALUES('legacy','Existing challenge','Description','Forensics','hard',200,'hmac','salt','/uploads/existing.zip','existing.zip')")
        .execute(&database).await.unwrap();
    database
        .execute(include_str!("../../migrations/0004_challenge_websites.sql"))
        .await
        .unwrap();
    let row: (String, i64, String, String, String, Option<String>) = sqlx::query_as(
        "SELECT title,points,flag_hmac,file_path,file_name,challenge_url FROM challenges WHERE id='legacy'")
        .fetch_one(&database).await.unwrap();
    assert_eq!(
        row,
        (
            "Existing challenge".into(),
            200,
            "hmac".into(),
            "/uploads/existing.zip".into(),
            "existing.zip".into(),
            None
        )
    );
}
