use super::*;
use crate::{csrf, services::dynamic_scoring::tests::fixture, session};
use axum::{
    body::Body,
    http::{header, HeaderMap, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn identity(state: &AppState, id: &str, admin: bool, ferris: bool) -> (String, String) {
    sqlx::query("INSERT OR IGNORE INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
        VALUES (?, ?, ?, 'unused', ?, ?)").bind(id).bind(id).bind(id).bind(admin).bind(ferris)
        .execute(&state.db).await.unwrap();
    let sealed = session::create(&state.db, &state.keys, 24, id)
        .await
        .unwrap();
    let user = session::authenticate(&state.db, &state.keys, &sealed)
        .await
        .unwrap()
        .unwrap();
    let token = csrf::issue(&state.keys, &user.session_id);
    (format!("{}={sealed}", session::COOKIE_NAME), token)
}

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    token: Option<&str>,
    body: &str,
) -> (StatusCode, HeaderMap, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    if let Some(t) = token {
        req = req.header("x-csrf-token", t);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        String::from_utf8(body.collect().await.unwrap().to_bytes().to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn guests_players_and_ordinary_admins_cannot_read_or_mutate_scoring() {
    let (state, _dir) = fixture(true, 1).await;
    let app = crate::app::build(state.clone());
    let player = identity(&state, "u0", false, false).await;
    let admin = identity(&state, "admin", true, false).await;
    for (cookie, token) in [
        (None, None),
        (Some(player.0.as_str()), Some(player.1.as_str())),
        (Some(admin.0.as_str()), Some(admin.1.as_str())),
    ] {
        let hidden = send(&app, "GET", "/does-not-exist", cookie, None, "").await;
        let page = send(&app, "GET", "/admin/scoring", cookie, None, "").await;
        assert_eq!(page.0, StatusCode::NOT_FOUND);
        assert_eq!(page.2, hidden.2, "do not disclose operator page existence");
        assert_eq!(page.1.get(header::CACHE_CONTROL).unwrap(), "no-store");
        for path in ["/admin/scoring", "/admin/scoring/challenges/easy/points"] {
            let result = send(
                &app,
                "POST",
                path,
                cookie,
                token,
                "points=150&reason=attack&action=mode&enabled=0&revision=0",
            )
            .await;
            assert!(matches!(
                result.0,
                StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
            ));
        }
    }
    let forged = send(
        &app,
        "GET",
        "/admin/scoring?is_admin=1&is_ferris=1",
        Some("ferris_session=forged.signature"),
        None,
        "",
    )
    .await;
    assert_eq!(forged.0, StatusCode::NOT_FOUND);
    let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(audit, 0);
    let points: i64 = sqlx::query_scalar(
        "SELECT anchor_units FROM challenge_pricing WHERE challenge_id = 'easy'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(points, 100 * market::SCALE);
    let team_page = send(&app, "GET", "/challenges", Some(&player.0), None, "").await;
    assert!(!team_page.2.contains("/admin/scoring"));
}

#[tokio::test]
async fn operator_edits_require_csrf_and_page_renders_with_live_controls() {
    let (state, _dir) = fixture(true, 1).await;
    let app = crate::app::build(state.clone());
    let operator = identity(&state, "operator", true, true).await;
    let player = identity(&state, "u0", false, false).await;
    let page = send(&app, "GET", "/admin/scoring", Some(&operator.0), None, "").await;
    assert_eq!(page.0, StatusCode::OK, "{}", page.2);
    assert!(page.2.contains("Save easy rule") && page.2.contains("Set points"));
    for path in ["/admin/scoring", "/admin/scoring/challenges/easy/points"] {
        for token in [None, Some("invalid"), Some(player.1.as_str())] {
            let denied = send(
                &app,
                "POST",
                path,
                Some(&operator.0),
                token,
                "points=140&reason=test&action=mode&enabled=0&revision=0",
            )
            .await;
            assert_eq!(denied.0, StatusCode::FORBIDDEN);
        }
    }
    let saved = send(
        &app,
        "POST",
        "/admin/scoring/challenges/easy/points",
        Some(&operator.0),
        Some(&operator.1),
        "points=140&reason=Adjust+future+awards",
    )
    .await;
    assert_eq!(saved.0, StatusCode::SEE_OTHER, "{}", saved.2);
    let rules = "action=rule&revision=0&reason=Tune+during+event&difficulty=easy&starting_points=100&minimum_points=40&maximum_points=150&quiet_minutes=20&recovery_per_minute=2&drop_percent=20";
    let saved = send(
        &app,
        "POST",
        "/admin/scoring",
        Some(&operator.0),
        Some(&operator.1),
        rules,
    )
    .await;
    assert_eq!(saved.0, StatusCode::SEE_OTHER, "{}", saved.2);
    let stale = send(
        &app,
        "POST",
        "/admin/scoring",
        Some(&operator.0),
        Some(&operator.1),
        rules,
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    let mode = send(
        &app,
        "POST",
        "/admin/scoring",
        Some(&operator.0),
        Some(&operator.1),
        "action=mode&revision=1&reason=Switch&enabled=0",
    )
    .await;
    assert_eq!(
        mode.0,
        StatusCode::CONFLICT,
        "mode is fixed after first round"
    );
}

#[tokio::test]
async fn quotes_are_authenticated_scoped_and_show_zero_earned_points() {
    let (state, _dir) = fixture(true, 2).await;
    let app = crate::app::build(state.clone());
    let player = identity(&state, "u0", false, false).await;
    let anonymous = send(&app, "GET", "/api/challenges/points", None, None, "").await;
    assert_eq!(anonymous.0, StatusCode::SEE_OTHER);
    let quote = send(
        &app,
        "GET",
        "/api/challenges/points",
        Some(&player.0),
        None,
        "",
    )
    .await;
    assert_eq!(quote.0, StatusCode::OK);
    assert_eq!(quote.1.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let json: serde_json::Value = serde_json::from_str(&quote.2).unwrap();
    assert_eq!(json["challenges"].as_array().unwrap().len(), 3);
    assert!(
        !quote.2.contains("hidden")
            && !quote.2.contains("flag")
            && !quote.2.contains("drop_percent")
    );
    sqlx::query("UPDATE team_challenge_pool SET repeat_penalty = 1000 WHERE team_id = 't0'")
        .execute(&state.db)
        .await
        .unwrap();
    let saved = send(
        &app,
        "POST",
        "/challenges/easy/submit",
        Some(&player.0),
        Some(&player.1),
        "flag=ferris%7Bcorrect%7D",
    )
    .await;
    assert_eq!(saved.0, StatusCode::SEE_OTHER, "{}", saved.2);
    let detail = send(&app, "GET", "/challenges/easy", Some(&player.0), None, "").await;
    assert_eq!(detail.0, StatusCode::OK);
    assert!(
        detail.2.contains("0 points</strong>") && detail.2.contains("0 pts earned"),
        "{}",
        detail.2
    );
    let quote = send(
        &app,
        "GET",
        "/api/challenges/points",
        Some(&player.0),
        None,
        "",
    )
    .await;
    let json: serde_json::Value = serde_json::from_str(&quote.2).unwrap();
    let easy = json["challenges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["id"] == "easy")
        .unwrap();
    assert_eq!(easy["points_awarded"], 0);
    sqlx::query("UPDATE teams SET is_locked = 0 WHERE id = 't0'")
        .execute(&state.db)
        .await
        .unwrap();
    let quote = send(
        &app,
        "GET",
        "/api/challenges/points",
        Some(&player.0),
        None,
        "",
    )
    .await;
    let json: serde_json::Value = serde_json::from_str(&quote.2).unwrap();
    assert!(json["challenges"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn mode_can_be_selected_before_the_event_only() {
    let (state, _dir) = crate::state::test_support::state().await;
    let operator = identity(&state, "operator", true, true).await;
    let app = crate::app::build(state.clone());
    let saved = send(
        &app,
        "POST",
        "/admin/scoring",
        Some(&operator.0),
        Some(&operator.1),
        "action=mode&revision=0&reason=Enable+for+contest&enabled=1",
    )
    .await;
    assert_eq!(saved.0, StatusCode::SEE_OTHER);
    let mut conn = state.db.acquire().await.unwrap();
    assert!(market::settings(&mut conn).await.unwrap().enabled);
    let page = send(&app, "GET", "/admin/scoring", Some(&operator.0), None, "").await;
    assert!(page.2.contains("Save mode"));
}
