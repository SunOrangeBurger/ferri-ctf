use std::{collections::HashMap, sync::Arc};

use axum::{extract::State, response::{IntoResponse, Response}};
use minijinja::context;
use serde::Serialize;
use sqlx::FromRow;

use crate::{errors::AppResult, middleware::auth::AdminUser, services::{rounds, scoring}, state::AppState, templates::UserView};

#[derive(Serialize, FromRow)]
struct Question {
    team_id: String,
    title: String,
    category: String,
    difficulty: String,
    is_active: bool,
    first_assigned: Option<String>,
    assigned_rounds: Option<String>,
    solved_at: Option<String>,
    solver: Option<String>,
    solve_round: Option<i64>,
    points_awarded: Option<i64>,
    attempts: i64,
    hint_cost: i64,
    elapsed: Option<String>,
}

#[derive(Serialize, FromRow)]
struct Adjustment {
    team_id: String,
    delta: i64,
    reason: String,
    created_at: String,
    admin_name: String,
}

#[derive(Serialize)]
struct TeamReport {
    score: scoring::ScoreboardEntry,
    questions: Vec<Question>,
    adjustments: Vec<Adjustment>,
    attempts: i64,
    hint_cost: i64,
    adjustment_total: i64,
    unsolved_active: usize,
}

pub async fn page(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
) -> AppResult<Response> {
    // Use the same ranking service as the public leaderboard, without its cache.
    let scoreboard = scoring::compute_scoreboard(&state.db).await?;
    let questions = sqlx::query_as::<_, Question>(
        "WITH pool AS (
            SELECT team_id,challenge_id,MIN(assigned_at) first_assigned,
                   GROUP_CONCAT(DISTINCT round_number) assigned_rounds
            FROM team_challenge_pool GROUP BY team_id,challenge_id
         ), pairs AS (
            SELECT team_id,challenge_id FROM pool UNION SELECT team_id,challenge_id FROM solves
         ), attempts AS (
            SELECT team_id,challenge_id,COUNT(*) n FROM submissions GROUP BY team_id,challenge_id
         ), used_hints AS (
            SELECT hu.team_id,h.challenge_id,SUM(h.point_cost) cost FROM hint_usage hu
            JOIN hints h ON h.id=hu.hint_id GROUP BY hu.team_id,h.challenge_id
         )
         SELECT p.team_id,c.title,c.category,c.difficulty,c.is_active,
                pool.first_assigned,pool.assigned_rounds,s.solved_at,u.display_name solver,
                s.round_number solve_round,s.points_awarded,
                COALESCE(a.n,0) attempts,COALESCE(h.cost,0) hint_cost,
                CASE WHEN s.solved_at IS NOT NULL AND pool.first_assigned IS NOT NULL
                     THEN CAST(CAST(MAX(0,ROUND((julianday(s.solved_at)-julianday(pool.first_assigned))*86400)) AS INTEGER) AS TEXT)
                END elapsed
         FROM pairs p JOIN challenges c ON c.id=p.challenge_id
         LEFT JOIN pool ON pool.team_id=p.team_id AND pool.challenge_id=p.challenge_id
         LEFT JOIN solves s ON s.team_id=p.team_id AND s.challenge_id=p.challenge_id
         LEFT JOIN users u ON u.id=s.user_id
         LEFT JOIN attempts a ON a.team_id=p.team_id AND a.challenge_id=p.challenge_id
         LEFT JOIN used_hints h ON h.team_id=p.team_id AND h.challenge_id=p.challenge_id
         ORDER BY s.solved_at IS NULL,s.solved_at,c.title"
    ).fetch_all(&state.db).await?;
    let adjustments = sqlx::query_as::<_, Adjustment>(
        "SELECT a.team_id,a.delta,a.reason,a.created_at,u.display_name admin_name
         FROM score_adjustments a JOIN users u ON u.id=a.admin_id ORDER BY a.created_at"
    ).fetch_all(&state.db).await?;
    let mut question_map: HashMap<String, Vec<Question>> = HashMap::new();
    for mut q in questions {
        q.elapsed = q.elapsed.and_then(|v| v.parse::<i64>().ok())
            .map(|s| format!("{}h {:02}m {:02}s", s/3600, (s/60)%60, s%60));
        question_map.entry(q.team_id.clone()).or_default().push(q);
    }
    let mut adjustment_map: HashMap<String, Vec<Adjustment>> = HashMap::new();
    for a in adjustments { adjustment_map.entry(a.team_id.clone()).or_default().push(a); }
    let reports: Vec<TeamReport> = scoreboard.into_iter().map(|score| {
        let questions = question_map.remove(&score.team_id).unwrap_or_default();
        let adjustments = adjustment_map.remove(&score.team_id).unwrap_or_default();
        TeamReport {
            attempts: questions.iter().map(|q| q.attempts).sum(),
            hint_cost: questions.iter().map(|q| q.hint_cost).sum(),
            adjustment_total: adjustments.iter().map(|a| a.delta).sum(),
            unsolved_active: questions.iter().filter(|q| q.is_active && q.solved_at.is_none()).count(),
            score, questions, adjustments,
        }
    }).collect();
    let active = rounds::get_active_round(&state.db).await?;
    let html = state.templates.render("admin/leaderboard.html", context! {
        user => UserView::from(&admin), reports => reports, active_round => active,
        updated_at => chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })?;
    let mut response = html.into_response();
    response.headers_mut().insert(axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::{Request, StatusCode}};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn private_leaderboard_renders_history_and_blocks_participants() {
        let (state, _dir) = crate::services::dynamic_scoring::tests::fixture(false, 2).await;
        sqlx::query("INSERT INTO users (id,username,display_name,password_hash,is_admin)
            VALUES ('admin','admin','Ordinary Admin','unused',1)").execute(&state.db).await.unwrap();
        sqlx::query("UPDATE team_challenge_pool SET assigned_at='2026-10-17 07:00:00' WHERE team_id='t0'")
            .execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO solves(team_id,challenge_id,user_id,round_number,points_awarded,solved_at)
            VALUES ('t0','easy','u0',1,123,'2026-10-17 07:05:00.000')")
            .execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO score_adjustments(id,team_id,admin_id,delta,reason,created_at)
            VALUES ('adjust','t0','admin',7,'Verified bonus','2026-10-17 07:06:00')")
            .execute(&state.db).await.unwrap();
        let direct_token = crate::session::create(&state.db,&state.keys,24,"admin").await.unwrap();
        let principal = crate::session::authenticate(&state.db,&state.keys,&direct_token).await.unwrap().unwrap();
        super::page(axum::extract::State(state.clone()),crate::middleware::auth::AdminUser(principal)).await.unwrap();
        let app = crate::app::build(state.clone());
        for id in [None,Some("u0"),Some("admin"),Some("operator")] {
            let mut request = Request::builder().uri("/admin/leaderboard").header("host","localhost");
            if let Some(id) = id {
                let token = crate::session::create(&state.db,&state.keys,24,id).await.unwrap();
                request = request.header("cookie",format!("{}={token}",crate::session::COOKIE_NAME));
            }
            let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
            if id == Some("admin") || id == Some("operator") {
                assert_eq!(response.status(),StatusCode::OK);
                assert_eq!(response.headers()["cache-control"],"no-store");
                let body = String::from_utf8(response.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
                for expected in ["Team 0","Team 1","Challenge easy","Solved","Unsolved","12:35:00 PM IST","0h 05m 00s","Verified bonus", "130"] {
                    assert!(body.contains(expected),"Missing {expected}");
                }
                assert!(!body.contains("flag_hmac"));
            } else { assert_eq!(response.status(),StatusCode::NOT_FOUND); }
        }
        // The public API must still expose only rank, team name and members.
        let response = app.oneshot(Request::builder().uri("/api/scoreboard").header("host","localhost")
            .body(Body::empty()).unwrap()).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entry = json["scoreboard"][0].as_object().unwrap();
        assert_eq!(entry.len(),3);
        assert!(entry.contains_key("rank") && entry.contains_key("team_name") && entry.contains_key("members"));
    }
}
