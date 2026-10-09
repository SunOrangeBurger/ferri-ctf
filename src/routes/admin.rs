use std::sync::Arc;

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Redirect, Response},
    Form,
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
    pub action: String, // "start" | "end" | "limit"
    pub round_number: Option<i64>,
    pub time_limit_minutes: Option<i64>,
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
    rounds_svc::start_event(&state, form.time_limit_minutes, &ferris.user_id).await?;
    scoring::invalidate_cache(&state).await;
    Ok(Redirect::to("/admin/rounds").into_response())
}

/// GET /admin/rounds
pub async fn rounds_page(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let rounds = sqlx::query_as::<_, Round>(
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at
         FROM rounds ORDER BY round_number DESC",
    )
    .fetch_all(&state.db)
    .await?;

    let event_state = sqlx::query_as::<_, EventState>(
        "SELECT id, scheduled_start, started_at FROM event_state WHERE id = 1",
    )
    .fetch_one(&state.db)
    .await?;

    let active_round = rounds_svc::get_active_round(&state.db).await?;

    let page = state.templates.render(
        "admin/rounds.html",
        context! {
            user => UserView::from(&ferris),
            csrf_token => ctx.token(&state.keys),
            rounds => rounds,
            event_state => event_state,
            active_round => active_round,
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
    rounds_svc::start_round(&state, form.time_limit_minutes, &ferris.user_id).await?;
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
    FerrisUser(ferris): FerrisUser,
    ctx: CsrfCtx,
) -> AppResult<Response> {
    let challenges = sqlx::query_as::<_, Challenge>(
        "SELECT id, title, description, category, difficulty, points,
                flag_hmac, flag_salt, file_path, file_name, is_active, created_at
         FROM challenges ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await?;

    let page = state.templates.render(
        "admin/challenges.html",
        context! {
            user => UserView::from(&ferris),
            csrf_token => ctx.token(&state.keys),
            challenges => challenges,
        },
    )?;
    Ok(page.into_response())
}

/// POST /admin/challenges/new (Multipart form)
pub async fn create_challenge_submit(
    State(state): State<Arc<AppState>>,
    FerrisUser(ferris): FerrisUser,
    mut multipart: Multipart,
) -> AppResult<Response> {
    let mut title = String::new();
    let mut description = String::new();
    let mut category = String::new();
    let mut difficulty = String::new();
    let mut points: i64 = 0;
    let mut flag = String::new();
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
                                 flag_hmac, flag_salt, file_path, file_name, is_active)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
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
    .execute(&state.db)
    .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
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

    rounds_svc::record_audit_log(
        &state.db,
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
    Path(challenge_id): Path<String>,
) -> AppResult<Response> {
    sqlx::query("UPDATE challenges SET is_active = 1 - is_active WHERE id = ?")
        .bind(&challenge_id)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
    Form(form): Form<BulkChallengeForm>,
) -> AppResult<Response> {
    let is_active = form.action == "activate";
    sqlx::query("UPDATE challenges SET is_active = ?")
        .bind(is_active)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
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
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
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
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
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
        &ferris.user_id,
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
    FerrisUser(ferris): FerrisUser,
    Path(hint_id): Path<String>,
) -> AppResult<Response> {
    sqlx::query("DELETE FROM hints WHERE id = ?")
        .bind(&hint_id)
        .execute(&state.db)
        .await?;

    rounds_svc::record_audit_log(
        &state.db,
        &ferris.user_id,
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
