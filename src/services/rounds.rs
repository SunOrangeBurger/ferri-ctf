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
pub async fn start_round(
    state: &AppState,
    time_limit_minutes: Option<i64>,
    admin_id: &str,
) -> AppResult<Round> {
    // Check if any round is currently active
    if let Some(active) = get_active_round(&state.db).await? {
        return Err(AppError::Conflict(format!(
            "Round {} is still active; end it before starting a new round",
            active.round_number
        )));
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

    // Assign pools for all locked teams for this round
    pool_service::assign_pools_for_round(
        &state.db,
        next_round,
        state.cfg.pool_size_per_team as usize,
        state.cfg.repeat_penalty_points,
    )
    .await?;

    // Audit log
    let details = format!("round_number: {next_round}, time_limit: {time_limit_minutes:?}");
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
) -> AppResult<Round> {
    let started: Option<String> = sqlx::query_scalar(
        "SELECT started_at FROM event_state WHERE id = 1",
    )
    .fetch_one(&state.db)
    .await?;

    if started.is_some() {
        return Err(AppError::Conflict("Event has already started".into()));
    }

    start_round(state, time_limit_minutes, admin_id).await
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
        let r1 = start_event(&state, Some(60), &admin_id).await.unwrap();
        assert_eq!(r1.round_number, 1);
        assert!(r1.auto_end_at.is_some());
        assert!(r1.ended_at.is_none());

        // Cannot start round 2 while round 1 is active
        let err = start_round(&state, None, &admin_id).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));

        // Change time limit
        let r1_mod = set_time_limit(&state, 1, Some(90), &admin_id).await.unwrap();
        assert_eq!(r1_mod.time_limit_minutes, Some(90));

        // End round 1
        let r1_end = end_round(&state, 1, &admin_id).await.unwrap();
        assert!(r1_end.ended_at.is_some());

        // Now start round 2
        let r2 = start_round(&state, None, &admin_id).await.unwrap();
        assert_eq!(r2.round_number, 2);
    }
}
