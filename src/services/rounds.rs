use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::Round,
    services::pool as pool_service,
    state::AppState,
};

/// Record an audit log entry.
pub async fn record_audit_log(
    pool: &SqlitePool,
    admin_id: &str,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    details: Option<&str>,
) -> AppResult<()> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO audit_log (id, admin_id, action, target_type, target_id, details)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(admin_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(details)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch the active round, if any.
/// Ignores rounds whose auto_end_at has already passed.
pub async fn get_active_round(pool: &SqlitePool) -> AppResult<Option<Round>> {
    let round = sqlx::query_as::<_, Round>(
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round
         FROM rounds
         WHERE ended_at IS NULL
           AND (auto_end_at IS NULL OR auto_end_at > strftime('%Y-%m-%d %H:%M:%f', 'now'))
         ORDER BY round_number DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(round)
}

/// Check and end any rounds whose auto_end_at has passed.
/// Returns a list of ended round numbers.
pub async fn check_auto_end_rounds(pool: &SqlitePool) -> AppResult<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT round_number FROM rounds
         WHERE ended_at IS NULL
           AND auto_end_at IS NOT NULL
           AND auto_end_at <= strftime('%Y-%m-%d %H:%M:%f', 'now')",
    )
    .fetch_all(pool)
    .await?;

    let mut ended = Vec::new();
    for r in rows {
        let rn: i64 = r.try_get("round_number")?;
        sqlx::query(
            "UPDATE rounds
             SET ended_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
             WHERE round_number = ? AND ended_at IS NULL",
        )
        .bind(rn)
        .execute(pool)
        .await?;
        ended.push(rn);
    }

    Ok(ended)
}

/// Start round N. Previous round must have ended.
/// Starts event / Round 1 if round_number == 1.
/// `selective_ids` — up to 5 challenge IDs marked `is_selective = 1` to assign for this round.
/// `force_reuse` — if true, allows selectives that were used in prior rounds.
/// `selective_only` — if true, the same five selectives are the entire pool for every team.
pub async fn start_round(
    state: &AppState,
    time_limit_minutes: Option<i64>,
    admin_id: &str,
    selective_ids: &[String],
    force_reuse: bool,
    selective_only: bool,
) -> AppResult<Round> {
    if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM rounds WHERE is_last_round = 1")
        .fetch_one(&state.db).await? > 0 {
        return Err(AppError::Conflict("LAST ROUND has already started; no further rounds can start".into()));
    }
    // Check if any round is currently active
    if let Some(active) = get_active_round(&state.db).await? {
        return Err(AppError::Conflict(format!(
            "Round {} is still active; end it before starting a new round",
            active.round_number
        )));
    }

    // Validate selective count
    if selective_ids.len() > 5 {
        return Err(AppError::BadRequest("At most 5 selectives can be chosen per round".into()));
    }
    if selective_only && !selective_ids.is_empty() && selective_ids.len() != 5 {
        return Err(AppError::BadRequest(
            "A selectives-only round requires exactly 5 selected challenges, or none for automatic selection".into(),
        ));
    }

    let next_round: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(round_number), 0) + 1 FROM rounds",
    )
    .fetch_one(&state.db)
    .await?;

    if let Some(mins) = time_limit_minutes {
        if mins <= 0 {
            return Err(AppError::BadRequest("Time limit must be positive".into()));
        }
    }

    // Validate each selective ID
    for sid in selective_ids {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM challenges WHERE id = ? AND is_active = 1 AND is_selective = 1",
        )
        .bind(sid)
        .fetch_optional(&state.db)
        .await?;

        if exists.is_none() {
            return Err(AppError::BadRequest(format!(
                "Challenge '{sid}' is not an active selective challenge"
            )));
        }

        if selective_only {
            let solved_by_team: Option<String> = sqlx::query_scalar(
                "SELECT t.name FROM solves s
                 JOIN teams t ON t.id = s.team_id
                 WHERE s.challenge_id = ? AND t.is_locked = 1 AND t.disbanded = 0
                 LIMIT 1",
            )
            .bind(sid)
            .fetch_optional(&state.db)
            .await?;
            if let Some(team_name) = solved_by_team {
                return Err(AppError::BadRequest(format!(
                    "Selective '{sid}' was already solved by team '{team_name}' and cannot be shared as a new special-round question"
                )));
            }
        }

        // Check once-used lockout (unless force_reuse is set)
        if !force_reuse && !selective_only {
            let already_used: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM round_selectives WHERE challenge_id = ?",
            )
            .bind(sid)
            .fetch_optional(&state.db)
            .await?;

            if already_used.is_some() {
                return Err(AppError::BadRequest(format!(
                    "Selective '{sid}' was already used in a prior round. \
                     Enable 'force reuse' to override."
                )));
            }
        }
    }

    // Resolve automatic selection before mutating event/round state so validation failures
    // cannot leave the event partially started.
    let mut sids = selective_ids.to_vec();
    if sids.is_empty() {
        let order = if selective_only { "RANDOM()" } else { "created_at ASC" };
        let solved_filter = if selective_only {
            "AND NOT EXISTS (
                SELECT 1 FROM solves s JOIN teams t ON t.id = s.team_id
                WHERE s.challenge_id = challenges.id AND t.is_locked = 1 AND t.disbanded = 0
            )"
        } else {
            ""
        };
        sids = sqlx::query_scalar::<_, String>(&format!(
            "SELECT id FROM challenges
             WHERE is_active = 1 AND is_selective = 1 {solved_filter}
             ORDER BY {order} LIMIT 5"
        ))
        .fetch_all(&state.db)
        .await?;
    }
    if selective_only && sids.len() != 5 {
        return Err(AppError::BadRequest(
            "At least 5 active selective challenges are required for a selectives-only round".into(),
        ));
    }

    // Round 1 start is the event start
    if next_round == 1 {
        sqlx::query(
            "UPDATE event_state
             SET started_at = COALESCE(started_at, strftime('%Y-%m-%d %H:%M:%f', 'now'))
             WHERE id = 1",
        )
        .execute(&state.db)
        .await?;

        // Lock all teams with >= TEAM_MIN_SIZE members
        crate::services::team::lock_all_teams(&state.db).await?;
    }

    let modifier = time_limit_minutes.map(|m| format!("+{m} minutes"));

    let round = match modifier {
        Some(mod_str) => {
            sqlx::query_as::<_, Round>(
                "INSERT INTO rounds (round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only)
                 VALUES (?, strftime('%Y-%m-%d %H:%M:%f', 'now'), ?, strftime('%Y-%m-%d %H:%M:%f', 'now', ?), NULL, ?)
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round",
            )
            .bind(next_round)
            .bind(time_limit_minutes)
            .bind(&mod_str)
            .bind(selective_only)
            .fetch_one(&state.db)
            .await?
        }
        None => {
            sqlx::query_as::<_, Round>(
                "INSERT INTO rounds (round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only)
                 VALUES (?, strftime('%Y-%m-%d %H:%M:%f', 'now'), NULL, NULL, NULL, ?)
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round",
            )
            .bind(next_round)
            .bind(selective_only)
            .fetch_one(&state.db)
            .await?
        }
    };

    // Persist the selected challenges for this round.
    for sid in &sids {
        sqlx::query(
            "INSERT OR IGNORE INTO round_selectives (round_number, challenge_id) VALUES (?, ?)",
        )
        .bind(next_round)
        .bind(sid)
        .execute(&state.db)
        .await?;
    }

    // Assign pools (standard + selectives) for all locked teams
    pool_service::assign_pools_for_round(
        &state.db,
        next_round,
        state.cfg.pool_size_per_team as usize,
        state.cfg.repeat_penalty_points,
    )
    .await?;

    // Audit log
    let sel_count = sids.len();
    let details = format!("round_number: {next_round}, time_limit: {time_limit_minutes:?}, selectives: {sel_count}, selective_only: {selective_only}");
    record_audit_log(
        &state.db,
        admin_id,
        "start_round",
        Some("round"),
        Some(&next_round.to_string()),
        Some(&details),
    )
    .await?;

    Ok(round)
}

/// Atomically start the final round with the entire challenge bank for every
/// locked team. Florida Man is the sole event-specific exclusion.
pub async fn start_last_round(
    state: &AppState,
    time_limit_minutes: Option<i64>,
    admin_id: &str,
) -> AppResult<Round> {
    if time_limit_minutes.is_some_and(|m| m <= 0) {
        return Err(AppError::BadRequest("Time limit must be positive".into()));
    }
    let mut tx = state.db.begin().await?;
    // Acquire the SQLite write lock before checking lifecycle state.
    sqlx::query("UPDATE event_state SET id = id WHERE id = 1").execute(&mut *tx).await?;
    let invalid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rounds WHERE is_last_round = 1 OR
         (ended_at IS NULL AND (auto_end_at IS NULL OR auto_end_at > strftime('%Y-%m-%d %H:%M:%f','now')))"
    ).fetch_one(&mut *tx).await?;
    if invalid > 0 {
        return Err(AppError::Conflict("End the active round first; LAST ROUND can only start once".into()));
    }
    let previous: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rounds")
        .fetch_one(&mut *tx).await?;
    if previous == 0 {
        return Err(AppError::BadRequest("Start the event before LAST ROUND".into()));
    }
    let round = sqlx::query_as::<_, Round>(
        "INSERT INTO rounds (round_number, time_limit_minutes, auto_end_at, is_last_round)
         SELECT COALESCE(MAX(round_number),0)+1, ?,
             CASE WHEN ? IS NULL THEN NULL ELSE strftime('%Y-%m-%d %H:%M:%f','now', ?) END, 1
         FROM rounds
         RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round"
    ).bind(time_limit_minutes).bind(time_limit_minutes)
        .bind(time_limit_minutes.map(|m| format!("+{m} minutes")))
        .fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE challenges SET is_active = CASE WHEN title='Florida Man' THEN 0 ELSE 1 END")
        .execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO last_round_challenges SELECT ?, c.id FROM challenges c
         WHERE c.title <> 'Florida Man'"
    ).bind(round.round_number).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO team_challenge_pool
         (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
         SELECT t.id,l.round_number,c.id,0,
             CASE WHEN settings.enabled=1 THEN rules.maximum_points ELSE c.points END
         FROM teams t CROSS JOIN last_round_challenges l
         JOIN challenges c ON c.id=l.challenge_id
         CROSS JOIN scoring_settings settings
         LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty=c.difficulty
         WHERE l.round_number=? AND t.is_locked=1 AND t.disbanded=0"
    ).bind(round.round_number).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_log (id,admin_id,action,target_type,target_id,details)
        VALUES (?,?,'start_last_round','round',?,'Every challenge assigned to every team; Florida Man omitted')")
        .bind(Uuid::new_v4().to_string()).bind(admin_id).bind(round.round_number.to_string())
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(round)
}

/// Share the most-exposed eligible team's pre-round question set with every team.
pub async fn start_backlog_round(
    state: &AppState,
    time_limit_minutes: Option<i64>,
    admin_id: &str,
) -> AppResult<Round> {
    if time_limit_minutes.is_some_and(|m| m <= 0) {
        return Err(AppError::BadRequest("Time limit must be positive".into()));
    }
    let mut tx = state.db.begin().await?;
    // Acquire the SQLite write lock before checking lifecycle state.
    sqlx::query("UPDATE event_state SET id = id WHERE id = 1").execute(&mut *tx).await?;
    let invalid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rounds WHERE is_last_round = 1 OR
         (ended_at IS NULL AND (auto_end_at IS NULL OR auto_end_at > strftime('%Y-%m-%d %H:%M:%f','now')))"
    ).fetch_one(&mut *tx).await?;
    if invalid > 0 {
        return Err(AppError::Conflict("End the active round first; no rounds can start after LAST ROUND".into()));
    }
    let previous: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rounds")
        .fetch_one(&mut *tx).await?;
    if previous == 0 {
        return Err(AppError::BadRequest("Start the event before Backlog Round".into()));
    }
    let round = sqlx::query_as::<_, Round>(
        "INSERT INTO rounds (round_number, time_limit_minutes, auto_end_at, is_backlog_round)
         SELECT COALESCE(MAX(round_number),0)+1, ?,
             CASE WHEN ? IS NULL THEN NULL ELSE strftime('%Y-%m-%d %H:%M:%f','now', ?) END, 1
         FROM rounds
         RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round"
    ).bind(time_limit_minutes).bind(time_limit_minutes)
        .bind(time_limit_minutes.map(|m| format!("+{m} minutes")))
        .fetch_one(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO backlog_round_challenges
         SELECT DISTINCT ?, p.challenge_id
         FROM team_challenge_pool p JOIN challenges c ON c.id=p.challenge_id
         WHERE p.round_number < ? AND c.is_active=1 AND p.team_id=(
             SELECT t.id FROM teams t JOIN team_challenge_pool history ON history.team_id=t.id
             WHERE t.is_locked=1 AND t.disbanded=0 AND history.round_number < ?
             GROUP BY t.id
             ORDER BY COUNT(DISTINCT history.challenge_id) DESC, t.created_at ASC, t.id ASC
             LIMIT 1
         )"
    ).bind(round.round_number).bind(round.round_number).bind(round.round_number)
        .execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO team_challenge_pool
         (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
         SELECT t.id,l.round_number,c.id,0,
             CASE WHEN settings.enabled=1 THEN rules.maximum_points ELSE c.points END
         FROM teams t CROSS JOIN backlog_round_challenges l
         JOIN challenges c ON c.id=l.challenge_id
         CROSS JOIN scoring_settings settings
         LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty=c.difficulty
         WHERE l.round_number=? AND t.is_locked=1 AND t.disbanded=0"
    ).bind(round.round_number).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_log (id,admin_id,action,target_type,target_id,details)
        VALUES (?,?,'start_backlog_round','round',?,'Shared active question set from the most-exposed team; existing solves preserved')")
        .bind(Uuid::new_v4().to_string()).bind(admin_id).bind(round.round_number.to_string())
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(round)
}

/// End an active round manually.
pub async fn end_round(
    state: &AppState,
    round_number: i64,
    admin_id: &str,
) -> AppResult<Round> {
    let round = sqlx::query_as::<_, Round>(
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round
         FROM rounds WHERE round_number = ?",
    )
    .bind(round_number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Round not found".into()))?;

    if round.ended_at.is_some() {
        return Err(AppError::Conflict("Round has already ended".into()));
    }

    let updated = sqlx::query_as::<_, Round>(
        "UPDATE rounds
         SET ended_at = strftime('%Y-%m-%d %H:%M:%f', 'now')
         WHERE round_number = ?
         RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round",
    )
    .bind(round_number)
    .fetch_one(&state.db)
    .await?;

    record_audit_log(
        &state.db,
        admin_id,
        "end_round",
        Some("round"),
        Some(&round_number.to_string()),
        None,
    )
    .await?;

    Ok(updated)
}

/// Set, update, or clear the time limit on an active round.
pub async fn set_time_limit(
    state: &AppState,
    round_number: i64,
    minutes: Option<i64>,
    admin_id: &str,
) -> AppResult<Round> {
    let round = sqlx::query_as::<_, Round>(
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round
         FROM rounds WHERE round_number = ?",
    )
    .bind(round_number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Round not found".into()))?;

    if round.ended_at.is_some() {
        return Err(AppError::Conflict("Cannot set time limit on an ended round".into()));
    }

    let updated = match minutes {
        Some(mins) => {
            if mins <= 0 {
                return Err(AppError::BadRequest("Time limit must be positive".into()));
            }

            // Check that started_at + mins is not already in the past
            let mod_str = format!("+{mins} minutes");
            let is_past: bool = sqlx::query_scalar(
                "SELECT (strftime('%Y-%m-%d %H:%M:%f', ?, ?) <= strftime('%Y-%m-%d %H:%M:%f', 'now'))",
            )
            .bind(&round.started_at)
            .bind(&mod_str)
            .fetch_one(&state.db)
            .await?;

            if is_past {
                return Err(AppError::BadRequest("New time limit would end in the past".into()));
            }

            sqlx::query_as::<_, Round>(
                "UPDATE rounds
                 SET time_limit_minutes = ?,
                     auto_end_at = strftime('%Y-%m-%d %H:%M:%f', started_at, ?)
                 WHERE round_number = ?
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round",
            )
            .bind(mins)
            .bind(&mod_str)
            .bind(round_number)
            .fetch_one(&state.db)
            .await?
        }
        None => {
            sqlx::query_as::<_, Round>(
                "UPDATE rounds
                 SET time_limit_minutes = NULL, auto_end_at = NULL
                 WHERE round_number = ?
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at, is_selective_only, is_last_round, is_backlog_round",
            )
            .bind(round_number)
            .fetch_one(&state.db)
            .await?
        }
    };

    let details = format!("limit: {minutes:?}");
    record_audit_log(
        &state.db,
        admin_id,
        "set_round_time_limit",
        Some("round"),
        Some(&round_number.to_string()),
        Some(&details),
    )
    .await?;

    Ok(updated)
}

/// Start the overall event: locks teams and begins Round 1.
pub async fn start_event(
    state: &AppState,
    time_limit_minutes: Option<i64>,
    admin_id: &str,
    selective_ids: &[String],
    force_reuse: bool,
    selective_only: bool,
) -> AppResult<Round> {
    let started: Option<String> = sqlx::query_scalar(
        "SELECT started_at FROM event_state WHERE id = 1",
    )
    .fetch_one(&state.db)
    .await?;

    if started.is_some() {
        return Err(AppError::Conflict("Event has already started".into()));
    }

    start_round(state, time_limit_minutes, admin_id, selective_ids, force_reuse, selective_only).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;

    #[tokio::test]
    async fn backlog_shares_most_exposed_team_set_and_preserves_solves() {
        let (state, _dir) = test_support::state().await;
        let admin = crate::services::auth::register(&state, "backlog_admin", "Admin", "Pass123456").await.unwrap();
        assert!(start_backlog_round(&state, None, &admin).await.is_err());
        for id in ["a", "b"] {
            sqlx::query("INSERT INTO users (id,username,display_name,password_hash) VALUES (?,?,?,'unused')")
                .bind(id).bind(id).bind(id).execute(&state.db).await.unwrap();
            sqlx::query("INSERT INTO teams (id,name,join_code,captain_id,is_locked) VALUES (?,?,?,?,1)")
                .bind(id).bind(id).bind(id).bind(id).execute(&state.db).await.unwrap();
        }
        sqlx::query("UPDATE event_state SET started_at=CURRENT_TIMESTAMP WHERE id=1").execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO rounds (round_number) VALUES (1)").execute(&state.db).await.unwrap();
        // Equal creation times exercise insertion order as the tie-breaker.
        for i in 1..=35 {
            let id = format!("q{i:02}");
            sqlx::query("INSERT INTO challenges (id,title,description,category,difficulty,points,flag_hmac,flag_salt,is_selective,is_active,created_at)
                VALUES (?,?,'','Misc','easy',50,'x','x',?,?, '2026-01-01 00:00:00')")
                .bind(&id).bind(&id).bind(i % 2 == 0).bind(i != 3).execute(&state.db).await.unwrap();
        }
        for (team, challenge) in [("a","q01"),("a","q02"),("a","q03"),("a","q31"),("b","q04")] {
            sqlx::query("INSERT INTO team_challenge_pool (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
                VALUES (?,1,?,0,50)").bind(team).bind(challenge).execute(&state.db).await.unwrap();
        }
        sqlx::query("INSERT INTO solves (team_id,challenge_id,user_id,round_number,points_awarded) VALUES ('a','q02','a',1,50)")
            .execute(&state.db).await.unwrap();
        assert!(start_backlog_round(&state, Some(20), &admin).await.is_err());
        end_round(&state, 1, &admin).await.unwrap();
        assert!(start_backlog_round(&state, Some(0), &admin).await.is_err());
        let round = start_backlog_round(&state, Some(20), &admin).await.unwrap();
        assert!(round.is_backlog_round);
        assert!(!round.is_last_round);
        assert!(round.auto_end_at.is_some());
        for team in ["a", "b"] {
            let ids: Vec<String> = sqlx::query_scalar("SELECT challenge_id FROM team_challenge_pool WHERE team_id=? AND round_number=2 ORDER BY challenge_id")
                .bind(team).fetch_all(&state.db).await.unwrap();
            assert_eq!(ids, vec!["q01", "q02", "q31"]);
        }
        let status: Vec<(String, bool)> = sqlx::query_as("SELECT p.team_id, s.challenge_id IS NOT NULL
            FROM team_challenge_pool p LEFT JOIN solves s ON s.team_id=p.team_id AND s.challenge_id=p.challenge_id
            WHERE p.round_number=2 AND p.challenge_id='q02' ORDER BY p.team_id")
            .fetch_all(&state.db).await.unwrap();
        assert_eq!(status, vec![("a".into(),true),("b".into(),false)]);
        // A later upload must not enter the frozen backlog pool through lazy assignment.
        sqlx::query("INSERT INTO challenges (id,title,description,category,difficulty,points,flag_hmac,flag_salt)
            VALUES ('later','later','','Misc','easy',50,'x','x')").execute(&state.db).await.unwrap();
        assert_eq!(pool_service::assign_round_pool(&state.db,"a",2,1,0).await.unwrap().len(),3);
        assert_eq!(pool_service::assign_selectives_for_team(&state.db,"a",2).await.unwrap().len(),3);
        end_round(&state, 2, &admin).await.unwrap();
        let normal = start_round(&state, None, &admin, &[], false, false).await.unwrap();
        assert_eq!(normal.round_number,3);
        end_round(&state,3,&admin).await.unwrap();
        let last = start_last_round(&state,None,&admin).await.unwrap();
        end_round(&state,last.round_number,&admin).await.unwrap();
        assert!(start_backlog_round(&state,None,&admin).await.is_err());
    }

    #[tokio::test]
    async fn last_round_assigns_entire_bank_to_every_team_except_florida_man() {
        let (state, _dir) = test_support::state().await;
        let admin = crate::services::auth::register(&state, "last_admin", "Admin", "Pass123456").await.unwrap();
        assert!(start_last_round(&state, None, &admin).await.is_err());
        for id in ["a", "b"] {
            sqlx::query("INSERT INTO users (id,username,display_name,password_hash) VALUES (?,?,?,'unused')")
                .bind(id).bind(id).bind(id).execute(&state.db).await.unwrap();
            sqlx::query("INSERT INTO teams (id,name,join_code,captain_id,is_locked) VALUES (?,?,?,?,1)")
                .bind(id).bind(id).bind(id).bind(id).execute(&state.db).await.unwrap();
        }
        sqlx::query("UPDATE event_state SET started_at=CURRENT_TIMESTAMP WHERE id=1").execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO rounds (round_number) VALUES (1)").execute(&state.db).await.unwrap();
        for (id, title, selective) in [
            ("seen","seen",0), ("unseen","unseen",1), ("own","own",0),
            ("zero_seen","zero_seen",0), ("zero_unseen","zero_unseen",0),
            ("inactive","inactive",0), ("florida","Florida Man",0),
        ] {
            sqlx::query("INSERT INTO challenges (id,title,description,category,difficulty,points,flag_hmac,flag_salt,is_selective,is_active)
                VALUES (?,?,'','Misc','easy',50,'x','x',?,?)")
                .bind(id).bind(title).bind(selective).bind(id != "inactive" && id != "florida").execute(&state.db).await.unwrap();
        }
        for id in ["seen", "zero_seen"] {
            sqlx::query("INSERT INTO team_challenge_pool (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
                VALUES ('a',1,?,0,50)").bind(id).execute(&state.db).await.unwrap();
        }
        for (team, id) in [("b","seen"),("b","unseen"),("a","own"),("b","inactive")] {
            sqlx::query("INSERT INTO solves (team_id,challenge_id,user_id,round_number,points_awarded) VALUES (?,?,?,1,50)")
                .bind(team).bind(id).bind(team).execute(&state.db).await.unwrap();
        }
        assert!(start_last_round(&state, Some(20), &admin).await.is_err());
        end_round(&state, 1, &admin).await.unwrap();
        assert!(start_last_round(&state, Some(0), &admin).await.is_err());
        let round = start_last_round(&state, Some(20), &admin).await.unwrap();
        assert!(round.is_last_round);
        assert!(round.auto_end_at.is_some());
        let ids: Vec<String> = sqlx::query_scalar("SELECT challenge_id FROM team_challenge_pool WHERE team_id='a' AND round_number=2 ORDER BY challenge_id")
            .fetch_all(&state.db).await.unwrap();
        assert_eq!(ids, vec!["inactive", "own", "seen", "unseen", "zero_seen", "zero_unseen"]);
        let ids: Vec<String> = sqlx::query_scalar("SELECT challenge_id FROM team_challenge_pool WHERE team_id='b' AND round_number=2 ORDER BY challenge_id")
            .fetch_all(&state.db).await.unwrap();
        assert_eq!(ids, vec!["inactive", "own", "seen", "unseen", "zero_seen", "zero_unseen"]);
        let active: Vec<(String, bool)> = sqlx::query_as(
            "SELECT id,is_active FROM challenges WHERE id IN ('inactive','florida') ORDER BY id")
            .fetch_all(&state.db).await.unwrap();
        assert_eq!(active, vec![("florida".into(),false),("inactive".into(),true)]);
        // Lazy assignment must preserve the special pool and cannot fall back to random selectives.
        pool_service::assign_round_pool(&state.db, "a", 2, 1, 0).await.unwrap();
        pool_service::assign_selectives_for_team(&state.db, "a", 2).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM team_challenge_pool WHERE team_id='a' AND round_number=2")
            .fetch_one(&state.db).await.unwrap();
        assert_eq!(count, 6);
        end_round(&state, 2, &admin).await.unwrap();
        assert!(start_last_round(&state, None, &admin).await.is_err());
        assert!(start_round(&state, None, &admin, &[], false, false).await.is_err());
    }

    #[tokio::test]
    async fn test_rounds_lifecycle() {
        let (state, _dir) = test_support::state().await;
        let admin_id = crate::services::auth::register(&state, "ferris_admin", "Ferris Admin", "Pass123456").await.unwrap();

        // Start event / Round 1
        let r1 = start_event(&state, Some(60), &admin_id, &[], false, false).await.unwrap();
        assert_eq!(r1.round_number, 1);
        assert!(r1.auto_end_at.is_some());
        assert!(r1.ended_at.is_none());

        // Cannot start round 2 while round 1 is active
        let err = start_round(&state, None, &admin_id, &[], false, false).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));

        // Change time limit
        let r1_mod = set_time_limit(&state, 1, Some(90), &admin_id).await.unwrap();
        assert_eq!(r1_mod.time_limit_minutes, Some(90));

        // End round 1
        let r1_end = end_round(&state, 1, &admin_id).await.unwrap();
        assert!(r1_end.ended_at.is_some());

        // Now start round 2
        let r2 = start_round(&state, None, &admin_id, &[], false, false).await.unwrap();
        assert_eq!(r2.round_number, 2);
    }

    #[tokio::test]
    async fn test_selectives_lifecycle_and_lockout() {
        let (state, _dir) = test_support::state().await;
        let admin_id = crate::services::auth::register(&state, "ferris_admin", "Ferris Admin", "Pass123456").await.unwrap();

        // Create 2 teams and lock them
        let u1 = crate::services::auth::register(&state, "user_one", "User One", "Pass123456").await.unwrap();
        let u2 = crate::services::auth::register(&state, "user_two", "User Two", "Pass123456").await.unwrap();
        let t1 = crate::services::team::create_team(&state.db, &u1, "Team Alpha", 30).await.unwrap();
        let t2 = crate::services::team::create_team(&state.db, &u2, "Team Beta", 30).await.unwrap();
        sqlx::query("UPDATE teams SET is_locked = 1 WHERE id IN (?, ?)").bind(&t1.id).bind(&t2.id).execute(&state.db).await.unwrap();

        // Insert 6 selective challenges and 1 non-selective challenge
        for i in 1..=6 {
            sqlx::query(
                "INSERT INTO challenges (id, title, description, category, difficulty, points, flag_hmac, flag_salt, is_active, is_selective)
                 VALUES (?, ?, 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 1)"
            )
            .bind(format!("sel-{i}"))
            .bind(format!("Selective {i}"))
            .execute(&state.db)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO challenges (id, title, description, category, difficulty, points, flag_hmac, flag_salt, is_active, is_selective)
             VALUES ('normal-1', 'Normal 1', 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 0)"
        )
        .execute(&state.db)
        .await
        .unwrap();

        // 1. More than 5 selectives rejected
        let six_ids: Vec<String> = (1..=6).map(|i| format!("sel-{i}")).collect();
        let err = start_event(&state, None, &admin_id, &six_ids, false, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 2. Non-selective challenge rejected as selective
        let bad_ids = vec!["normal-1".to_string()];
        let err = start_event(&state, None, &admin_id, &bad_ids, false, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 3. Start event with 5 selectives
        let five_ids: Vec<String> = (1..=5).map(|i| format!("sel-{i}")).collect();
        let r1 = start_event(&state, None, &admin_id, &five_ids, false, false).await.unwrap();
        assert_eq!(r1.round_number, 1);

        // Check round_selectives table has 5 entries
        let rs_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM round_selectives WHERE round_number = 1")
            .fetch_one(&state.db).await.unwrap();
        assert_eq!(rs_count, 5);

        // Check each team received exactly 2 selectives in team_challenge_pool
        for tid in [&t1.id, &t2.id] {
            let assigned_selectives: Vec<String> = sqlx::query_scalar(
                "SELECT challenge_id FROM team_challenge_pool
                 WHERE team_id = ? AND round_number = 1 AND challenge_id LIKE 'sel-%'",
            )
            .bind(tid)
            .fetch_all(&state.db)
            .await
            .unwrap();

            assert_eq!(assigned_selectives.len(), 2);
            // Verify they are within the 5 chosen selectives
            for sid in &assigned_selectives {
                assert!(five_ids.contains(sid));
            }

            // Verify repeat_penalty is 0
            let penalties: Vec<i64> = sqlx::query_scalar(
                "SELECT repeat_penalty FROM team_challenge_pool
                 WHERE team_id = ? AND round_number = 1 AND challenge_id LIKE 'sel-%'",
            )
            .bind(tid)
            .fetch_all(&state.db)
            .await
            .unwrap();
            assert_eq!(penalties, vec![0, 0]);
        }

        // End round 1
        end_round(&state, 1, &admin_id).await.unwrap();

        sqlx::query(
            "INSERT INTO solves
             (team_id, challenge_id, user_id, round_number, points_awarded, solved_at)
             VALUES (?, 'sel-1', ?, 1, 100, '2026-10-09 10:30:00.000')",
        )
        .bind(&t1.id)
        .bind(&u1)
        .execute(&state.db)
        .await
        .unwrap();

        // A shared selectives-only round cannot include a question already solved by
        // an eligible team, since all teams must receive the same five new questions.
        let err = start_round(&state, None, &admin_id, &five_ids, true, true)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 4. Try starting round 2 with a selective used in round 1 without force_reuse -> must fail!
        let reuse_ids = vec!["sel-1".to_string(), "sel-6".to_string()];
        let err = start_round(&state, None, &admin_id, &reuse_ids, false, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 5. Start round 2 with force_reuse = true -> succeeds!
        let r2 = start_round(&state, None, &admin_id, &reuse_ids, true, false).await.unwrap();
        assert_eq!(r2.round_number, 2);

        let rs2_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM round_selectives WHERE round_number = 2")
            .fetch_one(&state.db).await.unwrap();
        assert_eq!(rs2_count, 2);

        let t1_repeated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM team_challenge_pool
             WHERE team_id = ? AND round_number = 2 AND challenge_id = 'sel-1'",
        )
        .bind(&t1.id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(t1_repeated, 0, "solved selectives must not reappear");
    }

    #[tokio::test]
    async fn selectives_only_round_assigns_the_same_five_to_every_team() {
        let (state, _dir) = test_support::state().await;
        let admin_id = crate::services::auth::register(
            &state,
            "ferris_admin",
            "Ferris Admin",
            "Pass123456",
        )
        .await
        .unwrap();
        let user_id = crate::services::auth::register(
            &state,
            "player_one",
            "Player One",
            "Pass123456",
        )
        .await
        .unwrap();
        let team = crate::services::team::create_team(
            &state.db,
            &user_id,
            "Specialists",
            30,
        )
        .await
        .unwrap();

        for i in 1..=6 {
            sqlx::query(
                "INSERT INTO challenges
                 (id, title, description, category, difficulty, points, flag_hmac, flag_salt, is_active, is_selective)
                 VALUES (?, ?, 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 1)",
            )
            .bind(format!("special-{i}"))
            .bind(format!("Special {i}"))
            .execute(&state.db)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO challenges
             (id, title, description, category, difficulty, points, flag_hmac, flag_salt, is_active, is_selective)
             VALUES ('normal', 'Normal', 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let selected: Vec<String> = (1..=5).map(|i| format!("special-{i}")).collect();
        let round = start_event(&state, None, &admin_id, &selected, false, true)
            .await
            .unwrap();
        assert!(round.is_selective_only);

        let assigned: Vec<String> = sqlx::query_scalar(
            "SELECT challenge_id FROM team_challenge_pool
             WHERE team_id = ? AND round_number = 1 ORDER BY challenge_id",
        )
        .bind(&team.id)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(assigned, selected);
        assert!(!assigned.contains(&"normal".to_string()));

        // Re-running assignment, as late-loading routes do, remains idempotent.
        pool_service::assign_round_pool(&state.db, &team.id, 1, 15, 0)
            .await
            .unwrap();
        pool_service::assign_selectives_for_team(&state.db, &team.id, 1)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM team_challenge_pool WHERE team_id = ? AND round_number = 1",
        )
        .bind(&team.id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(count, 5);
    }
}
