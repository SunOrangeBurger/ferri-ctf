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
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at
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
pub async fn start_round(
    state: &AppState,
    time_limit_minutes: Option<i64>,
    admin_id: &str,
    selective_ids: &[String],
    force_reuse: bool,
) -> AppResult<Round> {
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

        // Check once-used lockout (unless force_reuse is set)
        if !force_reuse {
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
                "INSERT INTO rounds (round_number, started_at, time_limit_minutes, auto_end_at, ended_at)
                 VALUES (?, strftime('%Y-%m-%d %H:%M:%f', 'now'), ?, strftime('%Y-%m-%d %H:%M:%f', 'now', ?), NULL)
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at",
            )
            .bind(next_round)
            .bind(time_limit_minutes)
            .bind(&mod_str)
            .fetch_one(&state.db)
            .await?
        }
        None => {
            sqlx::query_as::<_, Round>(
                "INSERT INTO rounds (round_number, started_at, time_limit_minutes, auto_end_at, ended_at)
                 VALUES (?, strftime('%Y-%m-%d %H:%M:%f', 'now'), NULL, NULL, NULL)
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at",
            )
            .bind(next_round)
            .fetch_one(&state.db)
            .await?
        }
    };

    // Insert selected selectives for this round
    for sid in selective_ids {
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
    let sel_count = selective_ids.len();
    let details = format!("round_number: {next_round}, time_limit: {time_limit_minutes:?}, selectives: {sel_count}");
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

/// End an active round manually.
pub async fn end_round(
    state: &AppState,
    round_number: i64,
    admin_id: &str,
) -> AppResult<Round> {
    let round = sqlx::query_as::<_, Round>(
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at
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
         RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at",
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
        "SELECT round_number, started_at, time_limit_minutes, auto_end_at, ended_at
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
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at",
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
                 RETURNING round_number, started_at, time_limit_minutes, auto_end_at, ended_at",
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
) -> AppResult<Round> {
    let started: Option<String> = sqlx::query_scalar(
        "SELECT started_at FROM event_state WHERE id = 1",
    )
    .fetch_one(&state.db)
    .await?;

    if started.is_some() {
        return Err(AppError::Conflict("Event has already started".into()));
    }

    start_round(state, time_limit_minutes, admin_id, selective_ids, force_reuse).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;

    #[tokio::test]
    async fn test_rounds_lifecycle() {
        let (state, _dir) = test_support::state().await;
        let admin_id = crate::services::auth::register(&state, "ferris_admin", "Ferris Admin", "Pass123456").await.unwrap();

        // Start event / Round 1
        let r1 = start_event(&state, Some(60), &admin_id, &[], false).await.unwrap();
        assert_eq!(r1.round_number, 1);
        assert!(r1.auto_end_at.is_some());
        assert!(r1.ended_at.is_none());

        // Cannot start round 2 while round 1 is active
        let err = start_round(&state, None, &admin_id, &[], false).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));

        // Change time limit
        let r1_mod = set_time_limit(&state, 1, Some(90), &admin_id).await.unwrap();
        assert_eq!(r1_mod.time_limit_minutes, Some(90));

        // End round 1
        let r1_end = end_round(&state, 1, &admin_id).await.unwrap();
        assert!(r1_end.ended_at.is_some());

        // Now start round 2
        let r2 = start_round(&state, None, &admin_id, &[], false).await.unwrap();
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
        let err = start_event(&state, None, &admin_id, &six_ids, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 2. Non-selective challenge rejected as selective
        let bad_ids = vec!["normal-1".to_string()];
        let err = start_event(&state, None, &admin_id, &bad_ids, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 3. Start event with 5 selectives
        let five_ids: Vec<String> = (1..=5).map(|i| format!("sel-{i}")).collect();
        let r1 = start_event(&state, None, &admin_id, &five_ids, false).await.unwrap();
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

        // 4. Try starting round 2 with a selective used in round 1 without force_reuse -> must fail!
        let reuse_ids = vec!["sel-1".to_string(), "sel-6".to_string()];
        let err = start_round(&state, None, &admin_id, &reuse_ids, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 5. Start round 2 with force_reuse = true -> succeeds!
        let r2 = start_round(&state, None, &admin_id, &reuse_ids, true).await.unwrap();
        assert_eq!(r2.round_number, 2);

        let rs2_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM round_selectives WHERE round_number = 2")
            .fetch_one(&state.db).await.unwrap();
        assert_eq!(rs2_count, 2);
    }
}
