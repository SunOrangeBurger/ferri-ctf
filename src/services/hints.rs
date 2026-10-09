use sqlx::SqlitePool;

use crate::{
    errors::{AppError, AppResult},
    models::{Hint, HintView},
    services::rounds,
};

/// Request a hint for a challenge by a team member.
/// Any member's request unlocks it for the entire team.
/// Re-requesting an already unlocked hint returns the hint without charging again.
pub async fn request_hint(
    pool: &SqlitePool,
    team_id: &str,
    hint_id: &str,
) -> AppResult<Hint> {
    // 1. A round must be active
    let active_round = rounds::get_active_round(pool)
        .await?
        .ok_or_else(|| AppError::Forbidden("No round is currently active".into()))?;

    // 2. Team must be locked
    let is_locked: bool = sqlx::query_scalar(
        "SELECT is_locked FROM teams WHERE id = ? AND disbanded = 0",
    )
    .bind(team_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Team not found".into()))?;

    if !is_locked {
        return Err(AppError::Forbidden("Team must be locked to view challenges and hints".into()));
    }

    // 3. Find the hint and its challenge
    let hint = sqlx::query_as::<_, Hint>(
        "SELECT id, challenge_id, content, point_cost, created_at
         FROM hints WHERE id = ?",
    )
    .bind(hint_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Hint not found".into()))?;

    // 4. Challenge must be in team's pool for active round
    let in_pool: bool = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM team_challenge_pool
         WHERE team_id = ? AND challenge_id = ? AND round_number = ?",
    )
    .bind(team_id)
    .bind(&hint.challenge_id)
    .bind(active_round.round_number)
    .fetch_optional(pool)
    .await?
    .is_some();

    if !in_pool {
        return Err(AppError::NotFound("Challenge not in current round pool".into()));
    }

    // 5. Challenge must not already be solved
    let is_solved: bool = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM solves WHERE team_id = ? AND challenge_id = ?",
    )
    .bind(team_id)
    .bind(&hint.challenge_id)
    .fetch_optional(pool)
    .await?
    .is_some();

    if is_solved {
        return Err(AppError::Forbidden("Cannot request hints for an already solved challenge".into()));
    }

    // 6. Record hint usage if not already recorded
    sqlx::query(
        "INSERT OR IGNORE INTO hint_usage (team_id, hint_id)
         VALUES (?, ?)",
    )
    .bind(team_id)
    .bind(hint_id)
    .execute(pool)
    .await?;

    Ok(hint)
}

/// Get all hints for a challenge, with content populated only if unlocked by team.
pub async fn get_hints_for_challenge(
    pool: &SqlitePool,
    challenge_id: &str,
    team_id: &str,
) -> AppResult<Vec<HintView>> {
    let hints = sqlx::query_as::<_, Hint>(
        "SELECT id, challenge_id, content, point_cost, created_at
         FROM hints WHERE challenge_id = ?
         ORDER BY point_cost ASC, created_at ASC",
    )
    .bind(challenge_id)
    .fetch_all(pool)
    .await?;

    let unlocked_ids: Vec<String> = sqlx::query_scalar(
        "SELECT hint_id FROM hint_usage WHERE team_id = ?",
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?;

    let views = hints
        .into_iter()
        .map(|h| {
            let is_unlocked = unlocked_ids.contains(&h.id);
            HintView {
                id: h.id,
                challenge_id: h.challenge_id,
                point_cost: h.point_cost,
                is_unlocked,
                content: if is_unlocked { Some(h.content) } else { None },
            }
        })
        .collect();

    Ok(views)
}

/// Calculate total hint cost used by a team for a specific challenge.
pub async fn team_hint_cost_for_challenge(
    pool: &SqlitePool,
    team_id: &str,
    challenge_id: &str,
) -> AppResult<i64> {
    let cost: Option<i64> = sqlx::query_scalar(
        "SELECT SUM(h.point_cost)
         FROM hint_usage hu
         JOIN hints h ON h.id = hu.hint_id
         WHERE hu.team_id = ? AND h.challenge_id = ?",
    )
    .bind(team_id)
    .bind(challenge_id)
    .fetch_one(pool)
    .await?;

    Ok(cost.unwrap_or(0))
}
