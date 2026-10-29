use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use minijinja::context;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    middleware::{
        auth::{AdminUser, FerrisUser},
        csrf_guard::CsrfCtx,
    },
    models::{AuditLog, Challenge, EventState, Round},
    services::{
        files as files_svc,
        pool as pool_svc,
        rounds as rounds_svc,
        scoring,
        verifier,
    },
    state::AppState,
    templates::UserView,
};

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

#[derive(Deserialize)]
pub struct TeamActionForm {
    pub action: String, // "lock" | "unlock" | "disband" | "change_captain" | "remove_member" | "force_join"
    pub target_user_id: Option<String>,
    pub username: Option<String>,
}

#[derive(Deserialize)]
pub struct ScoreAdjForm {
    pub delta: i64,
    pub reason: String,
}

#[derive(Deserialize)]
pub struct UserActionForm {
    pub action: String, // "ban" | "unban"
}

#[derive(Deserialize)]
pub struct EventStartForm {
    pub action: Option<String>,
    pub time_limit_minutes: Option<i64>,
}

#[derive(Deserialize)]
pub struct RoundControlForm {
    #[serde(default)]
    pub action: Option<String>, // "start" | "end" | "limit"
    pub round_number: Option<i64>,
    pub time_limit_minutes: Option<i64>,
    /// Comma-separated selective challenge IDs (up to 5)
    pub selective_ids: Option<String>,
    /// Allow reusing selectives that were used in prior rounds
    pub force_reuse: Option<String>,
    /// Start a special round containing five selectives shared by every team.
    pub selective_only: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoundAdminView {
    pub round_number: i64,
    pub started_at: String,
    pub time_limit_minutes: Option<i64>,
    pub auto_end_at: Option<String>,
    pub ended_at: Option<String>,
    pub selectives: Vec<String>,
    pub is_selective_only: bool,
    pub is_last_round: bool,
    pub is_backlog_round: bool,
}

#[derive(Deserialize)]
pub struct EditChallengeForm {
    pub title: String,
    pub description: String,
    pub category: String,
    pub points: i64,
    pub is_active: Option<bool>,
}

#[derive(Deserialize)]
pub struct BulkChallengeForm {
    pub action: String, // "activate" | "deactivate"
}

#[derive(Deserialize)]
pub struct HintForm {
    pub content: String,
    pub point_cost: i64,
}

#[derive(Deserialize)]
pub struct AdminUserForm {
    pub username: String,
}

// =========================================================================
// ADMIN-LEVEL ROUTES (AdminUser)
// =========================================================================

/// GET /admin/dashboard
pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let teams_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teams WHERE disbanded = 0")
        .fetch_one(&state.db)
        .await?;

    let users_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await?;

    let solves_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM solves")
        .fetch_one(&state.db)
        .await?;

    let active_round = rounds_svc::get_active_round(&state.db).await?;

    let recent_audit = sqlx::query_as::<_, AuditLog>(
        "SELECT id, admin_id, action, target_type, target_id, details, created_at
         FROM audit_log ORDER BY created_at DESC LIMIT 10",
    )
    .fetch_all(&state.db)
    .await?;

    let page = state.templates.render(
        "admin/dashboard.html",
        context! {
            user => UserView::from(&user),
            csrf_token => ctx.token(&state.keys),
            teams_count => teams_count,
            users_count => users_count,
            solves_count => solves_count,
            active_round => active_round,
            recent_audit => recent_audit,
        },
    )?;
    Ok(page.into_response())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointMetrics {
    pub endpoint: String,
    pub count: usize,
    pub errors: usize,
    pub error_rate: f64,
    pub avg_ms: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsSummary {
    pub total_requests: usize,
    pub total_errors: usize,
    pub error_rate: f64,
    pub avg_ms: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsResponse {
    pub summary: AnalyticsSummary,
    pub endpoints: Vec<EndpointMetrics>,
    pub window_size: usize,
}

pub fn normalize_endpoint(path: &str) -> String {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segments.is_empty() || segments == [""] {
        return "/".to_string();
    }

    match segments.as_slice() {
        ["health"] => "/health".into(),
        ["login"] => "/login".into(),
        ["register"] => "/register".into(),
        ["logout"] => "/logout".into(),
        ["scoreboard"] => "/scoreboard".into(),
        ["api", "scoreboard"] => "/api/scoreboard".into(),
        ["api", "challenges", "points"] => "/api/challenges/points".into(),
        ["admin", "login"] => "/admin/login".into(),
        ["onboarding"] => "/onboarding".into(),
        ["team", "create"] => "/team/create".into(),
        ["team", "join"] => "/team/join".into(),
        ["team", "preview", _] => "/team/preview/:code".into(),
        ["team"] => "/team".into(),
        ["team", "leave"] => "/team/leave".into(),
        ["team", "kick"] => "/team/kick".into(),
        ["team", "transfer"] => "/team/transfer".into(),
        ["team", "rename"] => "/team/rename".into(),
        ["team", "disband"] => "/team/disband".into(),
        ["challenges"] => "/challenges".into(),
        ["challenges", _, "download"] => "/challenges/:id/download".into(),
        ["challenges", _, "submit"] => "/challenges/:id/submit".into(),
        ["challenges", _, "hint"] => "/challenges/:id/hint".into(),
        ["challenges", _] => "/challenges/:id".into(),
        ["profile"] => "/profile".into(),
        ["profile", "password"] => "/profile/password".into(),
        ["admin", "dashboard"] => "/admin/dashboard".into(),
        ["admin", "analytics"] => "/admin/analytics".into(),
        ["admin", "api", "analytics"] => "/admin/api/analytics".into(),
        ["admin", "teams"] => "/admin/teams".into(),
        ["admin", "teams", _, "action"] => "/admin/teams/:id/action".into(),
        ["admin", "teams", _, "score"] => "/admin/teams/:id/score".into(),
        ["admin", "users", _, "action"] => "/admin/users/:id/action".into(),
        ["admin", "export", _] => "/admin/export/:kind".into(),
        ["admin", "audit"] => "/admin/audit".into(),
        ["admin", "scoring"] => "/admin/scoring".into(),
        ["admin", "scoring", "challenges", _, "points"] => "/admin/scoring/challenges/:id/points".into(),
        ["admin", "event", "start"] => "/admin/event/start".into(),
        ["admin", "rounds"] => "/admin/rounds".into(),
        ["admin", "rounds", "start"] => "/admin/rounds/start".into(),
        ["admin", "rounds", "end"] => "/admin/rounds/end".into(),
        ["admin", "rounds", "limit"] => "/admin/rounds/limit".into(),
        ["admin", "challenges"] => "/admin/challenges".into(),
        ["admin", "challenges", "new"] => "/admin/challenges/new".into(),
        ["admin", "challenges", _, "edit"] => "/admin/challenges/:id/edit".into(),
        ["admin", "challenges", _, "toggle"] => "/admin/challenges/:id/toggle".into(),
        ["admin", "challenges", "bulk"] => "/admin/challenges/bulk".into(),
        ["admin", "challenges", _, "delete"] => "/admin/challenges/:id/delete".into(),
        ["admin", "challenges", _, "hints"] => "/admin/challenges/:id/hints".into(),
        ["admin", "hints", _, "edit"] => "/admin/hints/:id/edit".into(),
        ["admin", "hints", _, "delete"] => "/admin/hints/:id/delete".into(),
        ["admin", "admins"] => "/admin/admins".into(),
        ["admin", "admins", "add"] => "/admin/admins/add".into(),
        ["admin", "admins", _, "remove"] => "/admin/admins/:id/remove".into(),
        _ => {
            let normalized_segments: Vec<String> = segments
                .iter()
                .map(|s| {
                    if s.len() == 36 && s.contains('-') {
                        ":id".to_string()
                    } else if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                        ":id".to_string()
                    } else {
                        s.to_string()
                    }
                })
                .collect();
            format!("/{}", normalized_segments.join("/"))
        }
    }
}

pub fn calculate_stats(durations: &mut [u64], errors: usize, total: usize) -> (f64, u64, u64, u64, u64, f64) {
    if total == 0 {
        return (0.0, 0, 0, 0, 0, 0.0);
    }
    let error_rate = ((errors as f64 / total as f64) * 1000.0).round() / 10.0;
    if durations.is_empty() {
        return (0.0, 0, 0, 0, 0, error_rate);
    }
    durations.sort_unstable();
    let n = durations.len();
    let sum: u64 = durations.iter().sum();
    let avg = ((sum as f64 / n as f64) * 100.0).round() / 100.0;
    let max = durations[n - 1];

    let p50 = percentile(durations, 50.0);
    let p95 = percentile(durations, 95.0);
    let p99 = percentile(durations, 99.0);

    (avg, p50, p95, p99, max, error_rate)
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let rank = (n as f64 * p / 100.0).ceil() as usize;
    let idx = if rank == 0 { 0 } else { (rank - 1).min(n - 1) };
    sorted[idx]
}

/// GET /admin/analytics
pub async fn analytics_page(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let page = state.templates.render(
        "admin/analytics.html",
        context! {
            user => UserView::from(&user),
            csrf_token => ctx.token(&state.keys),
        },
    )?;
    Ok(page.into_response())
}

/// GET /admin/api/analytics
pub async fn analytics_json(
    State(state): State<Arc<AppState>>,
    AdminUser(_user): AdminUser,
) -> AppResult<Json<AnalyticsResponse>> {
    let metrics = {
        let store = state.metrics.read().unwrap_or_else(|e| e.into_inner());
        store.iter().cloned().collect::<Vec<_>>()
    };

    let total_requests = metrics.len();
    let mut total_errors = 0;
    let mut all_durations = Vec::with_capacity(total_requests);

    let mut groups: HashMap<String, (Vec<u64>, usize)> = HashMap::new();

    for m in &metrics {
        if m.status >= 400 {
            total_errors += 1;
        }
        all_durations.push(m.duration_ms);

        let endpoint = normalize_endpoint(&m.path);
        let entry = groups.entry(endpoint).or_insert_with(|| (Vec::new(), 0));
        entry.0.push(m.duration_ms);
        if m.status >= 400 {
            entry.1 += 1;
        }
    }

    let (s_avg, s_p50, s_p95, s_p99, s_max, s_err_rate) =
        calculate_stats(&mut all_durations, total_errors, total_requests);

    let summary = AnalyticsSummary {
        total_requests,
        total_errors,
        error_rate: s_err_rate,
        avg_ms: s_avg,
        p50_ms: s_p50,
        p95_ms: s_p95,
        p99_ms: s_p99,
        max_ms: s_max,
    };

    let mut endpoints = Vec::new();
    for (endpoint, (mut durations, errors)) in groups {
        let count = durations.len();
        let (avg, p50, p95, p99, max, error_rate) =
            calculate_stats(&mut durations, errors, count);
        endpoints.push(EndpointMetrics {
            endpoint,
            count,
            errors,
            error_rate,
            avg_ms: avg,
            p50_ms: p50,
            p95_ms: p95,
            p99_ms: p99,
            max_ms: max,
        });
    }

    endpoints.sort_by(|a, b| b.count.cmp(&a.count));

    Ok(Json(AnalyticsResponse {
        summary,
        endpoints,
        window_size: crate::state::MAX_METRICS_REQUESTS,
    }))
}

#[derive(Serialize)]
pub struct AdminTeamView {
    pub id: String,
    pub name: String,
    pub join_code: String,
    pub captain_id: String,
    pub is_locked: bool,
    pub disbanded: bool,
    pub created_at: String,
    pub member_names: Vec<String>,
}

/// GET /admin/teams
pub async fn teams_list(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    ctx: CsrfCtx,
    Query(query): Query<SearchQuery>,
) -> AppResult<Response> {
    let q = query.q.unwrap_or_default().trim().to_string();

    let team_rows = if q.is_empty() {
        sqlx::query(
            "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
             FROM teams ORDER BY created_at DESC",
        )
        .fetch_all(&state.db)
        .await?
    } else {
        let pattern = format!("%{q}%");
        sqlx::query(
            "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
             FROM teams WHERE name LIKE ? ORDER BY created_at DESC",
        )
        .bind(&pattern)
        .fetch_all(&state.db)
        .await?
    };

    let mut teams = Vec::with_capacity(team_rows.len());
    for r in team_rows {
        let tid: String = r.try_get("id")?;
        let members: Vec<String> = sqlx::query_scalar(
            "SELECT u.username FROM team_members tm
             JOIN users u ON u.id = tm.user_id
             WHERE tm.team_id = ?",
        )
        .bind(&tid)
        .fetch_all(&state.db)
        .await?;

        teams.push(AdminTeamView {
            id: tid,
            name: r.try_get("name")?,
            join_code: r.try_get("join_code")?,
            captain_id: r.try_get("captain_id")?,
            is_locked: r.try_get("is_locked")?,
            disbanded: r.try_get("disbanded")?,
            created_at: r.try_get("created_at")?,
            member_names: members,
        });
    }

    let page = state.templates.render(
        "admin/teams.html",
        context! {
            user => UserView::from(&user),
            csrf_token => ctx.token(&state.keys),
            teams => teams,
            search_query => q,
        },
    )?;
    Ok(page.into_response())
}

/// POST /admin/teams/:id/action
pub async fn team_action(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    Path(team_id): Path<String>,
    Form(form): Form<TeamActionForm>,
) -> AppResult<Response> {
    match form.action.as_str() {
        "lock" => {
            sqlx::query("UPDATE teams SET is_locked = 1 WHERE id = ?")
                .bind(&team_id)
                .execute(&state.db)
                .await?;

            if let Ok(Some(active_round)) = rounds_svc::get_active_round(&state.db).await {
                let _ = pool_svc::assign_round_pool(
                    &state.db,
                    &team_id,
                    active_round.round_number,
                    state.cfg.pool_size_per_team as usize,
                    state.cfg.repeat_penalty_points,
                )
                .await;
                let _ = pool_svc::assign_selectives_for_team(
                    &state.db,
                    &team_id,
                    active_round.round_number,
                )
                .await;
            }

            rounds_svc::record_audit_log(
                &state.db,
                &user.user_id,
                "admin_lock_team",
                Some("team"),
                Some(&team_id),
                None,
            )
            .await?;
        }
        "unlock" => {
            sqlx::query("UPDATE teams SET is_locked = 0 WHERE id = ?")
                .bind(&team_id)
                .execute(&state.db)
                .await?;
            rounds_svc::record_audit_log(
                &state.db,
                &user.user_id,
                "admin_unlock_team",
                Some("team"),
                Some(&team_id),
                None,
            )
            .await?;
        }
        "disband" => {
            sqlx::query("UPDATE teams SET disbanded = 1 WHERE id = ?")
                .bind(&team_id)
                .execute(&state.db)
                .await?;
            sqlx::query("DELETE FROM team_members WHERE team_id = ?")
                .bind(&team_id)
                .execute(&state.db)
                .await?;
            rounds_svc::record_audit_log(
                &state.db,
                &user.user_id,
                "admin_disband_team",
                Some("team"),
                Some(&team_id),
                None,
            )
            .await?;
        }
        "change_captain" => {
            if let Some(ref target_id) = form.target_user_id {
                let mut tx = state.db.begin().await?;
                sqlx::query("UPDATE teams SET captain_id = ? WHERE id = ?")
                    .bind(target_id)
                    .bind(&team_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("UPDATE team_members SET role = 'member' WHERE team_id = ?")
                    .bind(&team_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("UPDATE team_members SET role = 'captain' WHERE team_id = ? AND user_id = ?")
                    .bind(&team_id)
                    .bind(target_id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;

                rounds_svc::record_audit_log(
                    &state.db,
                    &user.user_id,
                    "admin_change_captain",
                    Some("team"),
                    Some(&team_id),
                    Some(target_id),
                )
                .await?;
            }
        }
        "remove_member" => {
            if let Some(ref target_id) = form.target_user_id {
                sqlx::query("DELETE FROM team_members WHERE team_id = ? AND user_id = ?")
                    .bind(&team_id)
                    .bind(target_id)
                    .execute(&state.db)
                    .await?;
                rounds_svc::record_audit_log(
                    &state.db,
                    &user.user_id,
                    "admin_remove_member",
                    Some("team"),
                    Some(&team_id),
                    Some(target_id),
                )
                .await?;
            }
        }
        "force_join" => {
            if let Some(ref username) = form.username {
                let uid: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
                    .bind(username)
                    .fetch_optional(&state.db)
                    .await?;
                if let Some(target_uid) = uid {
                    sqlx::query("DELETE FROM team_members WHERE user_id = ?")
                        .bind(&target_uid)
                        .execute(&state.db)
                        .await?;
                    sqlx::query(
                        "INSERT INTO team_members (team_id, user_id, role) VALUES (?, ?, 'member')",
                    )
                    .bind(&team_id)
                    .bind(&target_uid)
                    .execute(&state.db)
                    .await?;
                    rounds_svc::record_audit_log(
                        &state.db,
                        &user.user_id,
                        "admin_force_join",
                        Some("team"),
                        Some(&team_id),
                        Some(&target_uid),
                    )
                    .await?;
                }
            }
        }
        _ => {}
    }

    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/teams").into_response())
}

/// POST /admin/teams/:id/score — Manual score adjustment
pub async fn adjust_team_score(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    Path(team_id): Path<String>,
    Form(form): Form<ScoreAdjForm>,
) -> AppResult<Response> {
    let adj_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO score_adjustments (id, team_id, admin_id, delta, reason)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&adj_id)
    .bind(&team_id)
    .bind(&user.user_id)
    .bind(form.delta)
    .bind(&form.reason)
    .execute(&state.db)
    .await?;

    let details = format!("delta: {}, reason: {}", form.delta, form.reason);
    rounds_svc::record_audit_log(
        &state.db,
        &user.user_id,
        "score_adjust",
        Some("team"),
        Some(&team_id),
        Some(&details),
    )
    .await?;

    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/teams").into_response())
}

/// POST /admin/users/:id/action — Ban/unban
pub async fn user_action(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    Path(target_user_id): Path<String>,
    Form(form): Form<UserActionForm>,
) -> AppResult<Response> {
    let is_banned = form.action == "ban";

    sqlx::query("UPDATE users SET is_banned = ? WHERE id = ?")
        .bind(is_banned)
        .bind(&target_user_id)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &user.user_id,
        if is_banned { "ban_user" } else { "unban_user" },
        Some("user"),
        Some(&target_user_id),
        None,
    )
    .await?;

    Ok(Redirect::to("/admin/teams").into_response())
}

/// GET /admin/export/:kind — CSV export
pub async fn export_csv(
    State(state): State<Arc<AppState>>,
    AdminUser(_user): AdminUser,
    Path(kind): Path<String>,
) -> AppResult<Response> {
    let (csv_data, filename) = match kind.as_str() {
        "submissions" => {
            let rows = sqlx::query(
                "SELECT s.id, t.name AS team_name, u.username, c.title AS challenge_title,
                        s.is_correct, s.submitted_at
                 FROM submissions s
                 JOIN teams t ON t.id = s.team_id
                 JOIN users u ON u.id = s.user_id
                 JOIN challenges c ON c.id = s.challenge_id
                 ORDER BY s.submitted_at DESC",
            )
            .fetch_all(&state.db)
            .await?;

            let mut out = String::from("id,team_name,username,challenge_title,is_correct,submitted_at\n");
            for r in rows {
                let id: String = r.try_get("id")?;
                let team: String = r.try_get("team_name")?;
                let user: String = r.try_get("username")?;
                let chal: String = r.try_get("challenge_title")?;
                let correct: bool = r.try_get("is_correct")?;
                let ts: String = r.try_get("submitted_at")?;
                out.push_str(&format!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\"\n",
                    id.replace('"', "\"\""),
                    team.replace('"', "\"\""),
                    user.replace('"', "\"\""),
                    chal.replace('"', "\"\""),
                    if correct { 1 } else { 0 },
                    ts
                ));
            }
            (out, "submissions.csv")
        }
        "solves" => {
            let rows = sqlx::query(
                "SELECT t.name AS team_name, u.username, c.title AS challenge_title,
                        s.round_number, s.points_awarded, s.solved_at
                 FROM solves s
                 JOIN teams t ON t.id = s.team_id
                 JOIN users u ON u.id = s.user_id
                 JOIN challenges c ON c.id = s.challenge_id
                 ORDER BY s.solved_at DESC",
            )
            .fetch_all(&state.db)
            .await?;

            let mut out = String::from("team_name,username,challenge_title,round_number,points_awarded,solved_at\n");
            for r in rows {
                let team: String = r.try_get("team_name")?;
                let user: String = r.try_get("username")?;
                let chal: String = r.try_get("challenge_title")?;
                let round: i64 = r.try_get("round_number")?;
                let pts: i64 = r.try_get("points_awarded")?;
                let ts: String = r.try_get("solved_at")?;
                out.push_str(&format!(
                    "\"{}\",\"{}\",\"{}\",{},{},\"{}\"\n",
                    team.replace('"', "\"\""),
                    user.replace('"', "\"\""),
                    chal.replace('"', "\"\""),
                    round,
                    pts,
                    ts
                ));
            }
            (out, "solves.csv")
        }
        _ => return Err(AppError::NotFound("Export kind not found".into())),
    };

    let mut resp = Response::new(axum::body::Body::from(csv_data));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))?,
    );
    Ok(resp)
}

/// GET /admin/audit
pub async fn audit_log_page(
    State(state): State<Arc<AppState>>,
    AdminUser(user): AdminUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let rows = sqlx::query(
        "SELECT a.id, u.username AS admin_username, a.action, a.target_type, a.target_id, a.details, a.created_at
         FROM audit_log a
         JOIN users u ON u.id = a.admin_id
         ORDER BY a.created_at DESC LIMIT 100",
    )
    .fetch_all(&state.db)
    .await?;

    let mut entries = Vec::with_capacity(rows.len());
    for r in rows {
        entries.push(context! {
            id => r.try_get::<String, _>("id")?,
            admin_username => r.try_get::<String, _>("admin_username")?,
            action => r.try_get::<String, _>("action")?,
            target_type => r.try_get::<Option<String>, _>("target_type")?,
            target_id => r.try_get::<Option<String>, _>("target_id")?,
            details => r.try_get::<Option<String>, _>("details")?,
            created_at => r.try_get::<String, _>("created_at")?,
        });
    }

    let page = state.templates.render(
        "admin/audit.html",
        context! {
            user => UserView::from(&user),
            csrf_token => ctx.token(&state.keys),
            entries => entries,
        },
    )?;
    Ok(page.into_response())
}

// =========================================================================
// FERRIS-LEVEL ROUTES (FerrisUser)
// =========================================================================

/// POST /admin/event/start
pub async fn start_event_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<RoundControlForm>,
) -> AppResult<Response> {
    let selective_ids = parse_selective_ids(&form.selective_ids);
    let force_reuse = form.force_reuse.as_deref().map(|v| v == "true" || v == "on" || v == "1").unwrap_or(false);
    let selective_only = checkbox(&form.selective_only);
    rounds_svc::start_event(&state, form.time_limit_minutes, &ferris.user_id, &selective_ids, force_reuse, selective_only).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// Parse comma-separated selective IDs from form field.
fn parse_selective_ids(raw: &Option<String>) -> Vec<String> {
    raw.as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn checkbox(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|v| matches!(v, "true" | "on" | "1"))
}

/// GET /admin/rounds
pub async fn rounds_page(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let rounds_raw = sqlx::query_as::<_, Round>(
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round
         FROM rounds ORDER BY round_number DESC",
    )
    .fetch_all(&state.db)
    .await?;

    let selective_assignments = sqlx::query(
        "SELECT rs.round_number, c.title
         FROM round_selectives rs
         JOIN challenges c ON c.id = rs.challenge_id
         ORDER BY rs.round_number DESC, c.title ASC",
    )
    .fetch_all(&state.db)
    .await?;

    let mut selectives_by_round: HashMap<i64, Vec<String>> = HashMap::new();
    for row in selective_assignments {
        let r_num: i64 = row.try_get("round_number")?;
        let title: String = row.try_get("title")?;
        selectives_by_round.entry(r_num).or_default().push(title);
    }

    let last_round_started = rounds_raw.iter().any(|r| r.is_last_round);
    let rounds: Vec<RoundAdminView> = rounds_raw
        .into_iter()
        .map(|r| {
            let selectives = selectives_by_round.remove(&r.round_number).unwrap_or_default();
            RoundAdminView {
                round_number: r.round_number,
                started_at: r.started_at,
                time_limit_minutes: r.time_limit_minutes,
                auto_end_at: r.auto_end_at,
                ended_at: r.ended_at,
                selectives,
                is_selective_only: r.is_selective_only,
                is_last_round: r.is_last_round,
                is_backlog_round: r.is_backlog_round,
            }
        })
        .collect();

    let event_state = sqlx::query_as::<_, EventState>(
        "SELECT id, scheduled_start, started_at FROM event_state WHERE id = 1",
    )
    .fetch_one(&state.db)
    .await?;

    let active_round = rounds_svc::get_active_round(&state.db).await?;

    // Available selectives: active, is_selective, NOT already used in any round
    let available_selectives = sqlx::query_as::<_, Challenge>(
        "SELECT id, title, description, category, difficulty, points,
                flag_hmac, flag_salt, file_path, file_name, is_active, is_selective, created_at
         FROM challenges
         WHERE is_selective = 1 AND is_active = 1
           AND id NOT IN (SELECT challenge_id FROM round_selectives)
         ORDER BY title ASC",
    )
    .fetch_all(&state.db)
    .await?;

    // Used selectives (for "force reuse" display)
    let used_selectives = sqlx::query_as::<_, Challenge>(
        "SELECT id, title, description, category, difficulty, points,
                flag_hmac, flag_salt, file_path, file_name, is_active, is_selective, created_at
         FROM challenges
         WHERE is_selective = 1
           AND id IN (SELECT challenge_id FROM round_selectives)
         ORDER BY title ASC",
    )
    .fetch_all(&state.db)
    .await?;

    let page = state.templates.render(
        "admin/rounds.html",
        context! {
            user => UserView::from(&ferris),
            csrf_token => ctx.token(&state.keys),
            rounds => rounds,
            last_round_started => last_round_started,
            event_state => event_state,
            active_round => active_round,
            available_selectives => available_selectives,
            used_selectives => used_selectives,
        },
    )?;
    Ok(page.into_response())
}

/// POST /admin/rounds/start
pub async fn start_next_round_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<RoundControlForm>,
) -> AppResult<Response> {
    let selective_ids = parse_selective_ids(&form.selective_ids);
    let force_reuse = form.force_reuse.as_deref().map(|v| v == "true" || v == "on" || v == "1").unwrap_or(false);
    let selective_only = checkbox(&form.selective_only);
    rounds_svc::start_round(&state, form.time_limit_minutes, &ferris.user_id, &selective_ids, force_reuse, selective_only).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// POST /admin/rounds/last
pub async fn start_last_round_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<RoundControlForm>,
) -> AppResult<Response> {
    rounds_svc::start_last_round(&state, form.time_limit_minutes, &ferris.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// POST /admin/rounds/backlog
pub async fn start_backlog_round_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<RoundControlForm>,
) -> AppResult<Response> {
    rounds_svc::start_backlog_round(&state, form.time_limit_minutes, &ferris.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// POST /admin/rounds/end
pub async fn end_round_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<RoundControlForm>,
) -> AppResult<Response> {
    let rn = form.round_number.ok_or_else(|| AppError::BadRequest("Missing round number".into()))?;
    rounds_svc::end_round(&state, rn, &ferris.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// POST /admin/rounds/limit
pub async fn set_round_limit_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<RoundControlForm>,
) -> AppResult<Response> {
    let rn = form.round_number.ok_or_else(|| AppError::BadRequest("Missing round number".into()))?;
    rounds_svc::set_time_limit(&state, rn, form.time_limit_minutes, &ferris.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// GET /admin/challenges
pub async fn challenges_admin_page(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let challenges = sqlx::query_as::<_, Challenge>(
        "SELECT id, title, description, category, difficulty, points,
                flag_hmac, flag_salt, file_path, file_name, is_active, is_selective, created_at
         FROM challenges ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await?;

    let page = state.templates.render(
        "admin/challenges.html",
        context! {
            user => UserView::from(&admin),
            csrf_token => ctx.token(&state.keys),
            challenges => challenges,
        },
    )?;
    Ok(page.into_response())
}

/// POST /admin/challenges/new (Multipart form)
pub async fn create_challenge_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    mut multipart: Multipart,
) -> AppResult<Response> {
    let mut title = String::new();
    let mut description = String::new();
    let mut category = String::new();
    let mut difficulty = String::new();
    let mut points: i64 = 0;
    let mut flag = String::new();
    let mut is_selective = false;
    let mut file_data: Option<(String, Vec<u8>)> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(e.to_string()))? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "title" => title = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?,
            "description" => description = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?,
            "category" => category = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?,
            "difficulty" => difficulty = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?,
            "points" => {
                let pts_str = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?;
                points = pts_str.trim().parse::<i64>().unwrap_or(0);
            }
            "flag" => flag = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?,
            "is_selective" => {
                let val = field.text().await.map_err(|e| AppError::BadRequest(e.to_string()))?;
                is_selective = val == "on" || val == "1" || val == "true";
            }
            "file" => {
                let filename = field.file_name().map(|s| s.to_string());
                let bytes = field.bytes().await.map_err(|e| AppError::BadRequest(e.to_string()))?;
                if !bytes.is_empty() {
                    file_data = Some((filename.unwrap_or_else(|| "challenge.zip".into()), bytes.to_vec()));
                }
            }
            _ => {}
        }
    }

    if title.trim().is_empty() || flag.trim().is_empty() {
        return Err(AppError::BadRequest("Title and flag are required".into()));
    }

    let diff_norm = difficulty.trim().to_lowercase();
    if !matches!(diff_norm.as_str(), "easy" | "medium" | "hard") {
        return Err(AppError::BadRequest("Difficulty must be easy, medium, or hard".into()));
    }

    let challenge_id = Uuid::new_v4().to_string();
    let flag_salt = verifier::generate_salt();
    let flag_hmac = verifier::hash_flag(&state.keys.flag, &flag_salt, &flag);

    let (stored_path, safe_file_name) = match file_data {
        Some((fname, bytes)) => {
            let (path, safe_name) = files_svc::validate_and_store_zip_bytes(
                Some(&fname),
                &bytes,
                state.cfg.max_upload_bytes,
            )
            .await?;
            (Some(path), Some(safe_name))
        }
        None => (None, None),
    };

    sqlx::query(
        "INSERT INTO challenges (id, title, description, category, difficulty, points,
                                 flag_hmac, flag_salt, file_path, file_name, is_active, is_selective)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)",
    )
    .bind(&challenge_id)
    .bind(title.trim())
    .bind(description.trim())
    .bind(category.trim())
    .bind(&diff_norm)
    .bind(points)
    .bind(&flag_hmac)
    .bind(&flag_salt)
    .bind(stored_path)
    .bind(safe_file_name)
    .bind(is_selective)
    .execute(&state.db)
    .await?;

    // If a round is actively running and this is not a selective challenge,
    // add it to the active round's pool for all locked teams so participants can see and solve it immediately.
    if let Ok(Some(active_round)) = rounds_svc::get_active_round(&state.db).await {
        if !is_selective && !active_round.is_selective_only && !active_round.is_last_round && !active_round.is_backlog_round {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO team_challenge_pool
                 (team_id, round_number, challenge_id, repeat_penalty, opportunity_points)
                 SELECT t.id, ?, c.id, 0,
                        CASE WHEN settings.enabled = 1 THEN rules.maximum_points ELSE c.points END
                 FROM teams t
                 CROSS JOIN challenges c
                 CROSS JOIN scoring_settings settings
                 LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty = c.difficulty
                 WHERE t.is_locked = 1 AND t.disbanded = 0 AND c.id = ?",
            )
            .bind(active_round.round_number)
            .bind(&challenge_id)
            .execute(&state.db)
            .await;
        }
    }

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "create_challenge",
        Some("challenge"),
        Some(&challenge_id),
        Some(&title),
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/challenges/:id/edit
pub async fn edit_challenge_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(challenge_id): Path<String>,
    Form(form): Form<EditChallengeForm>,
) -> AppResult<Response> {
    // Note: difficulty is NOT editable per spec 4 line 324
    sqlx::query(
        "UPDATE challenges
         SET title = ?, description = ?, category = ?, points = ?, is_active = COALESCE(?, is_active)
         WHERE id = ?",
    )
    .bind(form.title.trim())
    .bind(form.description.trim())
    .bind(form.category.trim())
    .bind(form.points)
    .bind(form.is_active)
    .bind(&challenge_id)
    .execute(&state.db)
    .await?;

    if form.is_active == Some(true) {
        if let Ok(Some(active_round)) = rounds_svc::get_active_round(&state.db).await {
            let is_non_sel: Option<bool> = sqlx::query_scalar(
                "SELECT (is_selective = 0) FROM challenges WHERE id = ?"
            )
            .bind(&challenge_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);

            if is_non_sel == Some(true) && !active_round.is_selective_only && !active_round.is_last_round && !active_round.is_backlog_round {
                let _ = sqlx::query(
                    "INSERT OR IGNORE INTO team_challenge_pool
                     (team_id, round_number, challenge_id, repeat_penalty, opportunity_points)
                     SELECT t.id, ?, c.id, 0,
                            CASE WHEN settings.enabled = 1 THEN rules.maximum_points ELSE c.points END
                     FROM teams t
                     CROSS JOIN challenges c
                     CROSS JOIN scoring_settings settings
                     LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty = c.difficulty
                     WHERE t.is_locked = 1 AND t.disbanded = 0 AND c.id = ?",
                )
                .bind(active_round.round_number)
                .bind(&challenge_id)
                .execute(&state.db)
                .await;
            }
        }
    }

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "edit_challenge",
        Some("challenge"),
        Some(&challenge_id),
        None,
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/challenges/:id/toggle
pub async fn toggle_challenge_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(challenge_id): Path<String>,
) -> AppResult<Response> {
    sqlx::query("UPDATE challenges SET is_active = 1 - is_active WHERE id = ?")
        .bind(&challenge_id)
        .execute(&state.db)
        .await?;

    if let Ok(Some(active_round)) = rounds_svc::get_active_round(&state.db).await {
        let is_active_and_non_sel: Option<bool> = sqlx::query_scalar(
            "SELECT (is_active = 1 AND is_selective = 0) FROM challenges WHERE id = ?"
        )
        .bind(&challenge_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

        if is_active_and_non_sel == Some(true) && !active_round.is_selective_only && !active_round.is_last_round && !active_round.is_backlog_round {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO team_challenge_pool
                 (team_id, round_number, challenge_id, repeat_penalty, opportunity_points)
                 SELECT t.id, ?, c.id, 0,
                        CASE WHEN settings.enabled = 1 THEN rules.maximum_points ELSE c.points END
                 FROM teams t
                 CROSS JOIN challenges c
                 CROSS JOIN scoring_settings settings
                 LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty = c.difficulty
                 WHERE t.is_locked = 1 AND t.disbanded = 0 AND c.id = ?",
            )
            .bind(active_round.round_number)
            .bind(&challenge_id)
            .execute(&state.db)
            .await;
        }
    }

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "toggle_challenge",
        Some("challenge"),
        Some(&challenge_id),
        None,
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/challenges/bulk
pub async fn bulk_challenge_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Form(form): Form<BulkChallengeForm>,
) -> AppResult<Response> {
    let is_active = form.action == "activate";
    sqlx::query("UPDATE challenges SET is_active = ?")
        .bind(is_active)
        .execute(&state.db)
        .await?;

    if is_active {
        if let Ok(Some(active_round)) = rounds_svc::get_active_round(&state.db).await {
            if !active_round.is_selective_only && !active_round.is_last_round && !active_round.is_backlog_round {
                let _ = sqlx::query(
                    "INSERT OR IGNORE INTO team_challenge_pool
                     (team_id, round_number, challenge_id, repeat_penalty, opportunity_points)
                     SELECT t.id, ?, c.id, 0,
                            CASE WHEN settings.enabled = 1 THEN rules.maximum_points ELSE c.points END
                     FROM teams t
                     CROSS JOIN challenges c
                     CROSS JOIN scoring_settings settings
                     LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty = c.difficulty
                     WHERE t.is_locked = 1 AND t.disbanded = 0 AND c.is_active = 1 AND c.is_selective = 0",
                )
                .bind(active_round.round_number)
                .execute(&state.db)
                .await;
            }
        }
    }

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        if is_active { "bulk_activate_challenges" } else { "bulk_deactivate_challenges" },
        Some("challenges"),
        None,
        None,
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/challenges/:id/delete
pub async fn delete_challenge_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(challenge_id): Path<String>,
) -> AppResult<Response> {
    // Delete file if attached
    let path_row: Option<String> = sqlx::query_scalar("SELECT file_path FROM challenges WHERE id = ?")
        .bind(&challenge_id)
        .fetch_optional(&state.db)
        .await?;

    if let Some(path) = path_row {
        let _ = files_svc::delete_challenge_file(&path).await;
    }

    sqlx::query("DELETE FROM challenges WHERE id = ?")
        .bind(&challenge_id)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "delete_challenge",
        Some("challenge"),
        Some(&challenge_id),
        None,
    )
    .await?;

    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/challenges/:id/hints
pub async fn add_hint_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(challenge_id): Path<String>,
    Form(form): Form<HintForm>,
) -> AppResult<Response> {
    let hint_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO hints (id, challenge_id, content, point_cost)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&hint_id)
    .bind(&challenge_id)
    .bind(form.content.trim())
    .bind(form.point_cost)
    .execute(&state.db)
    .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "add_hint",
        Some("hint"),
        Some(&hint_id),
        Some(&format!("cost: {}", form.point_cost)),
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/hints/:id/edit
pub async fn edit_hint_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(hint_id): Path<String>,
    Form(form): Form<HintForm>,
) -> AppResult<Response> {
    sqlx::query("UPDATE hints SET content = ?, point_cost = ? WHERE id = ?")
        .bind(form.content.trim())
        .bind(form.point_cost)
        .bind(&hint_id)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "edit_hint",
        Some("hint"),
        Some(&hint_id),
        None,
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

/// POST /admin/hints/:id/delete
pub async fn delete_hint_submit(
    State(state): State<Arc<AppState>>,
    AdminUser(admin): AdminUser,
    Path(hint_id): Path<String>,
) -> AppResult<Response> {
    sqlx::query("DELETE FROM hints WHERE id = ?")
        .bind(&hint_id)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &admin.user_id,
        "delete_hint",
        Some("hint"),
        Some(&hint_id),
        None,
    )
    .await?;

    Ok(Redirect::to("/admin/challenges").into_response())
}

#[derive(Serialize)]
pub struct AdminViewRow {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub is_ferris: bool,
    pub created_at: String,
}

/// GET /admin/admins
pub async fn admins_page(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let rows = sqlx::query(
        "SELECT id, username, display_name, is_ferris, created_at
         FROM users WHERE is_admin = 1 ORDER BY is_ferris DESC, created_at ASC",
    )
    .fetch_all(&state.db)
    .await?;

    let mut admins = Vec::with_capacity(rows.len());
    for r in rows {
        admins.push(AdminViewRow {
            id: r.try_get("id")?,
            username: r.try_get("username")?,
            display_name: r.try_get("display_name")?,
            is_ferris: r.try_get::<i64, _>("is_ferris")? != 0,
            created_at: r.try_get("created_at")?,
        });
    }

    let page = state.templates.render(
        "admin/admins.html",
        context! {
            user => UserView::from(&ferris),
            csrf_token => ctx.token(&state.keys),
            admins => admins,
        },
    )?;
    Ok(page.into_response())
}

/// POST /admin/admins/add
pub async fn add_admin_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<AdminUserForm>,
) -> AppResult<Response> {
    let result = sqlx::query("UPDATE users SET is_admin = 1 WHERE username = ?")
        .bind(form.username.trim())
        .execute(&state.db)
        .await?;

    if result.rows_affected() > 0 {
        rounds_svc::record_audit_log(
            &state.db,
            &ferris.user_id,
            "grant_admin",
            Some("user"),
            None,
            Some(&form.username),
        )
        .await?;
    }

    Ok(Redirect::to("/admin/admins").into_response())
}

/// POST /admin/admins/:id/remove
pub async fn remove_admin_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    Path(target_user_id): Path<String>,
) -> AppResult<Response> {
    // Ferris cannot be removed or demoted (spec 9.8)
    let result = sqlx::query("UPDATE users SET is_admin = 0 WHERE id = ? AND is_ferris = 0")
        .bind(&target_user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() > 0 {
        rounds_svc::record_audit_log(
            &state.db,
            &ferris.user_id,
            "revoke_admin",
            Some("user"),
            Some(&target_user_id),
            None,
        )
        .await?;
    }

    Ok(Redirect::to("/admin/admins").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_endpoint() {
        assert_eq!(normalize_endpoint("/"), "/");
        assert_eq!(normalize_endpoint("/login"), "/login");
        assert_eq!(normalize_endpoint("/register"), "/register");
        assert_eq!(normalize_endpoint("/challenges"), "/challenges");
        assert_eq!(normalize_endpoint("/challenges/5/submit"), "/challenges/:id/submit");
        assert_eq!(
            normalize_endpoint("/challenges/a1b2c3d4-e5f6-7890-abcd-ef1234567890/download"),
            "/challenges/:id/download"
        );
        assert_eq!(normalize_endpoint("/team/preview/7X3K9M2P"), "/team/preview/:code");
        assert_eq!(normalize_endpoint("/admin/teams/123/score"), "/admin/teams/:id/score");
    }

    #[test]
    fn test_calculate_stats() {
        let mut empty = vec![];
        let (avg, p50, p95, p99, max, err_rate) = calculate_stats(&mut empty, 0, 0);
        assert_eq!((avg, p50, p95, p99, max, err_rate), (0.0, 0, 0, 0, 0, 0.0));

        let mut durations: Vec<u64> = (1..=100).collect();
        let (avg, p50, p95, p99, max, err_rate) = calculate_stats(&mut durations, 5, 100);
        assert_eq!(avg, 50.5);
        assert_eq!(p50, 50);
        assert_eq!(p95, 95);
        assert_eq!(p99, 99);
        assert_eq!(max, 100);
        assert_eq!(err_rate, 5.0);
    }

    #[tokio::test]
    async fn test_analytics_routes_and_metrics() {
        use axum::body::Body;
        use axum::http::{header, Request, StatusCode};
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        use crate::{app, session, state::test_support};

        let (state, _dir) = test_support::state().await;
        let app = app::build(state.clone());

        let send_req = |req: Request<Body>| {
            let app = app.clone();
            async move {
                let resp = app.oneshot(req).await.unwrap();
                let (parts, body) = resp.into_parts();
                let bytes = body.collect().await.unwrap().to_bytes();
                (parts.status, String::from_utf8_lossy(&bytes).into_owned())
            }
        };

        let (s, _) = send_req(Request::builder().uri("/health").body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::OK);

        let (s, _) = send_req(Request::builder().uri("/login").body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::OK);

        let (s, _) = send_req(Request::builder().uri("/static/style.css").body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::OK);

        // Verify static request was NOT recorded in metrics
        {
            let store = state.metrics.read().unwrap();
            assert!(store.iter().any(|m| m.path == "/health"));
            assert!(store.iter().any(|m| m.path == "/login"));
            assert!(!store.iter().any(|m| m.path.starts_with("/static")));
        }

        // Unauthenticated access to /admin/api/analytics and /admin/analytics returns 404 (Admin gate)
        let (s, _) = send_req(Request::builder().uri("/admin/api/analytics").body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::NOT_FOUND);

        let (s, _) = send_req(Request::builder().uri("/admin/analytics").body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::NOT_FOUND);

        // Create an admin user and session
        sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
             VALUES ('adm1', 'admin_tester', 'Admin Tester', 'x', 1, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let session_val = session::create(&state.db, &state.keys, 24, "adm1").await.unwrap();
        let cookie_header = format!("{}={}", session::COOKIE_NAME, session_val);

        // Access /admin/analytics HTML page as admin
        let (s, body) = send_req(
            Request::builder()
                .uri("/admin/analytics")
                .header(header::COOKIE, &cookie_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("Live Server Analytics"));
        assert!(body.contains("setInterval(fetchMetrics, 3000)"));

        // Access /admin/api/analytics JSON as admin
        let (s, json_body) = send_req(
            Request::builder()
                .uri("/admin/api/analytics")
                .header(header::COOKIE, &cookie_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);

        let resp: AnalyticsResponse = serde_json::from_str(&json_body).unwrap();
        assert!(resp.summary.total_requests >= 2);
        assert!(resp.endpoints.iter().any(|e| e.endpoint == "/health"));
        assert!(resp.endpoints.iter().any(|e| e.endpoint == "/login"));
    }

    #[tokio::test]
    async fn test_admin_can_add_challenge() {
        use axum::body::Body;
        use axum::http::{header, Request, StatusCode};
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        use crate::{app, session, state::test_support};

        let (state, _dir) = test_support::state().await;
        let app = app::build(state.clone());

        // Create an ordinary admin (is_admin = 1, is_ferris = 0)
        sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
             VALUES ('admin_user_1', 'regular_admin', 'Regular Admin', 'x', 1, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let session_val = session::create(&state.db, &state.keys, 24, "admin_user_1").await.unwrap();
        let cookie_header = format!("{}={}", session::COOKIE_NAME, session_val);

        // 1. Regular admin can access /admin/challenges
        let req = Request::builder()
            .uri("/admin/challenges")
            .header(header::COOKIE, &cookie_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body_bytes);
        assert!(html.contains("Upload New Challenge"));

        // Extract CSRF token from page
        let marker = "name=\"csrf_token\" value=\"";
        let start = html.find(marker).expect("csrf token in page") + marker.len();
        let csrf_token = html[start..].split('"').next().unwrap();

        // 2. Regular admin can add a new challenge via multipart POST
        let boundary = "---------------------------boundary123";
        let body_content = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nAdmin Question 1\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\nCrypto\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"difficulty\"\r\n\r\neasy\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"points\"\r\n\r\n150\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"flag\"\r\n\r\nferris{{admin_flag}}\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"description\"\r\n\r\nTest description\r\n\
             --{boundary}--\r\n"
        );

        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/challenges/new?csrf_token={csrf_token}"))
            .header(header::COOKIE, &cookie_header)
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body_content))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        // Verify challenge was inserted in DB
        let chal: Option<String> = sqlx::query_scalar("SELECT title FROM challenges WHERE title = 'Admin Question 1'")
            .fetch_optional(&state.db)
            .await
            .unwrap();
        assert_eq!(chal.as_deref(), Some("Admin Question 1"));

        // Verify audit log recorded admin_user_1 as the creator
        let audit: Option<String> = sqlx::query_scalar("SELECT admin_id FROM audit_log WHERE action = 'create_challenge'")
            .fetch_optional(&state.db)
            .await
            .unwrap();
        assert_eq!(audit.as_deref(), Some("admin_user_1"));

        // 3. Test starting the event via POST /admin/event/start with NO action field in the form body
        // (verifies that missing action field does not fail deserialization)
        sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
             VALUES ('ferris_op', 'ferris_operator', 'Ferris', 'x', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let ferris_session = session::create(&state.db, &state.keys, 24, "ferris_op").await.unwrap();
        let ferris_cookie = format!("{}={}", session::COOKIE_NAME, ferris_session);

        // Create a team with a member so event start will lock the team
        sqlx::query("INSERT INTO teams (id, name, captain_id, join_code, is_locked) VALUES ('team_1', 'Team 1', 'admin_user_1', 'CODE1', 0)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO team_members (team_id, user_id, role) VALUES ('team_1', 'admin_user_1', 'captain')")
            .execute(&state.db)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/admin/rounds")
            .header(header::COOKIE, &ferris_cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let html_rounds = String::from_utf8_lossy(&body_bytes);
        let start_rounds = html_rounds.find(marker).expect("csrf token in rounds page") + marker.len();
        let ferris_csrf = html_rounds[start_rounds..].split('"').next().unwrap();

        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/event/start?csrf_token={ferris_csrf}"))
            .header(header::COOKIE, &ferris_cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("csrf_token={ferris_csrf}&time_limit_minutes=60")))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        // Verify round 1 is active
        let active_round = rounds_svc::get_active_round(&state.db).await.unwrap().expect("active round");
        assert_eq!(active_round.round_number, 1);

        // 4. Admin adds a challenge WHILE Round 1 is active
        let body_content2 = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nLive Mid-Round Question\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\nWeb\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"difficulty\"\r\n\r\nmedium\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"points\"\r\n\r\n250\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"flag\"\r\n\r\nferris{{live_round_flag}}\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"description\"\r\n\r\nLive round test\r\n\
             --{boundary}--\r\n"
        );

        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/challenges/new?csrf_token={csrf_token}"))
            .header(header::COOKIE, &cookie_header)
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body_content2))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        // Find the challenge ID
        let live_chal_id: String = sqlx::query_scalar("SELECT id FROM challenges WHERE title = 'Live Mid-Round Question'")
            .fetch_one(&state.db)
            .await
            .unwrap();

        // Verify that the mid-round challenge was immediately inserted into team_challenge_pool for team_1 in round 1!
        let pool_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM team_challenge_pool WHERE team_id = 'team_1' AND round_number = 1 AND challenge_id = ?",
        )
        .bind(&live_chal_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(pool_count, 1, "Mid-round challenge must appear in active round's team challenge pool");
    }
}
