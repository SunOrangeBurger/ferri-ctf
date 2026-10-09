use rand::{rngs::OsRng, Rng};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    config::{JOIN_CODE_LENGTH, TEAM_MAX_SIZE, TEAM_MIN_SIZE},
    errors::{AppError, AppResult},
    models::{Team, TeamMemberView, TeamWithMembers},
};

const CROCKFORD_CHARS: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const TEAM_NAME_MIN: usize = 2;
pub const TEAM_NAME_MAX: usize = 32;

/// Generate an 8-character Crockford Base32 join code.
pub fn generate_join_code() -> String {
    let mut rng = OsRng;
    let mut code = String::with_capacity(JOIN_CODE_LENGTH);
    for _ in 0..JOIN_CODE_LENGTH {
        let idx = rng.gen_range(0..CROCKFORD_CHARS.len());
        code.push(CROCKFORD_CHARS[idx] as char);
    }
    code
}

pub fn validate_team_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    let char_count = name.chars().count();
    if !(TEAM_NAME_MIN..=TEAM_NAME_MAX).contains(&char_count) {
        return Err(AppError::BadRequest(format!(
            "Team name must be between {TEAM_NAME_MIN} and {TEAM_NAME_MAX} characters"
        )));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(AppError::BadRequest(
            "Team name contains forbidden control characters".into(),
        ));
    }
    Ok(name.to_string())
}

/// Normalizes join code by trimming and uppercasing (Crockford base32 is case-insensitive).
pub fn normalize_join_code(raw: &str) -> String {
    raw.trim().to_ascii_uppercase()
}

/// Check if event has started and whether we are within the grace period.
/// Returns (event_started: bool, grace_period_active: bool)
pub async fn check_event_and_grace_status(
    pool: &SqlitePool,
    grace_period_minutes: i64,
) -> AppResult<(bool, bool)> {
    let row = sqlx::query(
        "SELECT started_at,
                (datetime('now') <= datetime(started_at, '+' || ? || ' minutes')) AS in_grace
         FROM event_state WHERE id = 1",
    )
    .bind(grace_period_minutes)
    .fetch_one(pool)
    .await?;

    let started_at: Option<String> = row.try_get("started_at")?;
    let in_grace: Option<bool> = row.try_get("in_grace")?;

    match started_at {
        None => Ok((false, false)),
        Some(_) => Ok((true, in_grace.unwrap_or(false))),
    }
}

/// Create a new team with creator as captain.
pub async fn create_team(
    pool: &SqlitePool,
    user_id: &str,
    raw_name: &str,
    grace_period_minutes: i64,
) -> AppResult<Team> {
    let team_name = validate_team_name(raw_name)?;

    // Check if user is already in a team
    let existing_member = sqlx::query("SELECT team_id FROM team_members WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    if existing_member.is_some() {
        return Err(AppError::Conflict("You are already in a team".into()));
    }

    // Check event state
    let (event_started, in_grace) = check_event_and_grace_status(pool, grace_period_minutes).await?;
    if event_started && !in_grace {
        return Err(AppError::Forbidden(
            "The event has started and the grace period has ended; cannot create new teams".into(),
        ));
    }

    // Check if name is taken (case-insensitive across active teams)
    let name_taken = sqlx::query("SELECT id FROM teams WHERE lower(name) = lower(?) AND disbanded = 0")
        .bind(&team_name)
        .fetch_optional(pool)
        .await?;
    if name_taken.is_some() {
        return Err(AppError::Conflict("A team with this name already exists".into()));
    }

    let team_id = Uuid::new_v4().to_string();

    // Generate unique join code
    let mut join_code = generate_join_code();
    for _ in 0..10 {
        let code_exists = sqlx::query("SELECT id FROM teams WHERE join_code = ?")
            .bind(&join_code)
            .fetch_optional(pool)
            .await?;
        if code_exists.is_none() {
            break;
        }
        join_code = generate_join_code();
    }

    // If event has started and we are in grace period, team locks immediately once valid (>= TEAM_MIN_SIZE)
    let should_lock = event_started && in_grace;

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO teams (id, name, join_code, captain_id, is_locked, disbanded)
         VALUES (?, ?, ?, ?, ?, 0)",
    )
    .bind(&team_id)
    .bind(&team_name)
    .bind(&join_code)
    .bind(user_id)
    .bind(should_lock)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO team_members (team_id, user_id, role)
         VALUES (?, ?, 'captain')",
    )
    .bind(&team_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let team = sqlx::query_as::<_, Team>(
        "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
         FROM teams WHERE id = ?",
    )
    .bind(&team_id)
    .fetch_one(pool)
    .await?;

    Ok(team)
}

/// Preview a team by join code before confirming join.
pub async fn preview_team_by_code(pool: &SqlitePool, raw_code: &str) -> AppResult<TeamWithMembers> {
    let code = normalize_join_code(raw_code);

    let team = sqlx::query_as::<_, Team>(
        "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
         FROM teams WHERE join_code = ? AND disbanded = 0",
    )
    .bind(&code)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Team not found".into()))?;

    let members = sqlx::query_as::<_, TeamMemberView>(
        "SELECT tm.user_id, u.username, u.display_name, tm.role, tm.joined_at
         FROM team_members tm
         JOIN users u ON u.id = tm.user_id
         WHERE tm.team_id = ?
         ORDER BY (CASE WHEN tm.role = 'captain' THEN 0 ELSE 1 END), tm.joined_at ASC",
    )
    .bind(&team.id)
    .fetch_all(pool)
    .await?;

    Ok(TeamWithMembers { team, members })
}

/// Join a team using join code.
pub async fn join_team(
    pool: &SqlitePool,
    user_id: &str,
    raw_code: &str,
    grace_period_minutes: i64,
) -> AppResult<Team> {
    let code = normalize_join_code(raw_code);

    // Check if user is already in a team
    let existing = sqlx::query("SELECT team_id FROM team_members WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    if existing.is_some() {
        return Err(AppError::Conflict("You are already in a team".into()));
    }

    let (event_started, in_grace) = check_event_and_grace_status(pool, grace_period_minutes).await?;
    if event_started && !in_grace {
        return Err(AppError::Forbidden(
            "Event has started and the grace period has ended; cannot join teams".into(),
        ));
    }

    let mut tx = pool.begin().await?;

    let team = sqlx::query_as::<_, Team>(
        "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
         FROM teams WHERE join_code = ? AND disbanded = 0",
    )
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Team not found".into()))?;

    if team.is_locked {
        return Err(AppError::Forbidden(
            "This team is locked and cannot be joined".into(),
        ));
    }

    let member_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM team_members WHERE team_id = ?",
    )
    .bind(&team.id)
    .fetch_one(&mut *tx)
    .await?;

    if member_count as usize >= TEAM_MAX_SIZE {
        return Err(AppError::Conflict("Team is already full".into()));
    }

    sqlx::query(
        "INSERT INTO team_members (team_id, user_id, role)
         VALUES (?, ?, 'member')",
    )
    .bind(&team.id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    // If event has started and grace period is active, lock team once it reaches TEAM_MIN_SIZE
    let new_count = member_count + 1;
    let should_lock = event_started && in_grace && (new_count as usize >= TEAM_MIN_SIZE);
    if should_lock {
        sqlx::query("UPDATE teams SET is_locked = 1 WHERE id = ?")
            .bind(&team.id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    let updated_team = sqlx::query_as::<_, Team>(
        "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
         FROM teams WHERE id = ?",
    )
    .bind(&team.id)
    .fetch_one(pool)
    .await?;

    Ok(updated_team)
}

/// Leave team. Pre-lock only. Captain cannot leave if another member remains.
pub async fn leave_team(pool: &SqlitePool, user_id: &str) -> AppResult<()> {
    let mut tx = pool.begin().await?;

    let membership = sqlx::query(
        "SELECT tm.team_id, tm.role, t.is_locked
         FROM team_members tm
         JOIN teams t ON t.id = tm.team_id
         WHERE tm.user_id = ? AND t.disbanded = 0",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("You are not in a team".into()))?;

    let team_id: String = membership.try_get("team_id")?;
    let role: String = membership.try_get("role")?;
    let is_locked: bool = membership.try_get("is_locked")?;

    if is_locked {
        return Err(AppError::Forbidden("Cannot leave a locked team".into()));
    }

    let other_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM team_members WHERE team_id = ? AND user_id != ?",
    )
    .bind(&team_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;

    if role == "captain" && other_count > 0 {
        return Err(AppError::Forbidden(
            "Captains cannot leave while another member remains. Transfer captaincy or disband the team.".into(),
        ));
    }

    sqlx::query("DELETE FROM team_members WHERE team_id = ? AND user_id = ?")
        .bind(&team_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    // If captain was the only member, disband the team
    if role == "captain" && other_count == 0 {
        sqlx::query("UPDATE teams SET disbanded = 1 WHERE id = ?")
            .bind(&team_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Kick member. Captain only, pre-lock only.
pub async fn kick_member(pool: &SqlitePool, captain_id: &str, target_user_id: &str) -> AppResult<()> {
    if captain_id == target_user_id {
        return Err(AppError::BadRequest("Captain cannot kick themselves".into()));
    }

    let mut tx = pool.begin().await?;

    let captain_membership = sqlx::query(
        "SELECT tm.team_id, t.is_locked
         FROM team_members tm
         JOIN teams t ON t.id = tm.team_id
         WHERE tm.user_id = ? AND tm.role = 'captain' AND t.disbanded = 0",
    )
    .bind(captain_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Forbidden("Only the captain can kick members".into()))?;

    let team_id: String = captain_membership.try_get("team_id")?;
    let is_locked: bool = captain_membership.try_get("is_locked")?;

    if is_locked {
        return Err(AppError::Forbidden("Cannot kick members from a locked team".into()));
    }

    let target_in_team = sqlx::query(
        "SELECT 1 FROM team_members WHERE team_id = ? AND user_id = ?",
    )
    .bind(&team_id)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if target_in_team.is_none() {
        return Err(AppError::NotFound("User is not in your team".into()));
    }

    sqlx::query("DELETE FROM team_members WHERE team_id = ? AND user_id = ?")
        .bind(&team_id)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Transfer captaincy to another member. Pre-lock only.
pub async fn transfer_captain(
    pool: &SqlitePool,
    captain_id: &str,
    target_user_id: &str,
) -> AppResult<()> {
    if captain_id == target_user_id {
        return Err(AppError::BadRequest("You are already the captain".into()));
    }

    let mut tx = pool.begin().await?;

    let captain_membership = sqlx::query(
        "SELECT tm.team_id, t.is_locked
         FROM team_members tm
         JOIN teams t ON t.id = tm.team_id
         WHERE tm.user_id = ? AND tm.role = 'captain' AND t.disbanded = 0",
    )
    .bind(captain_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Forbidden("Only the captain can transfer captaincy".into()))?;

    let team_id: String = captain_membership.try_get("team_id")?;
    let is_locked: bool = captain_membership.try_get("is_locked")?;

    if is_locked {
        return Err(AppError::Forbidden(
            "Cannot transfer captaincy of a locked team".into(),
        ));
    }

    let target_in_team = sqlx::query(
        "SELECT 1 FROM team_members WHERE team_id = ? AND user_id = ?",
    )
    .bind(&team_id)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if target_in_team.is_none() {
        return Err(AppError::NotFound("Target user is not in your team".into()));
    }

    // Spec §4 note: team_members.role and teams.captain_id must be updated together in one transaction.
    sqlx::query("UPDATE teams SET captain_id = ? WHERE id = ?")
        .bind(target_user_id)
        .bind(&team_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE team_members SET role = 'member' WHERE team_id = ? AND user_id = ?")
        .bind(&team_id)
        .bind(captain_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE team_members SET role = 'captain' WHERE team_id = ? AND user_id = ?")
        .bind(&team_id)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Rename team. Pre-lock only.
pub async fn rename_team(pool: &SqlitePool, captain_id: &str, raw_name: &str) -> AppResult<Team> {
    let new_name = validate_team_name(raw_name)?;

    let mut tx = pool.begin().await?;

    let captain_membership = sqlx::query(
        "SELECT tm.team_id, t.is_locked
         FROM team_members tm
         JOIN teams t ON t.id = tm.team_id
         WHERE tm.user_id = ? AND tm.role = 'captain' AND t.disbanded = 0",
    )
    .bind(captain_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Forbidden("Only the captain can rename the team".into()))?;

    let team_id: String = captain_membership.try_get("team_id")?;
    let is_locked: bool = captain_membership.try_get("is_locked")?;

    if is_locked {
        return Err(AppError::Forbidden("Cannot rename a locked team".into()));
    }

    let name_taken = sqlx::query(
        "SELECT id FROM teams WHERE lower(name) = lower(?) AND id != ? AND disbanded = 0",
    )
    .bind(&new_name)
    .bind(&team_id)
    .fetch_optional(&mut *tx)
    .await?;

    if name_taken.is_some() {
        return Err(AppError::Conflict("A team with this name already exists".into()));
    }

    sqlx::query("UPDATE teams SET name = ? WHERE id = ?")
        .bind(&new_name)
        .bind(&team_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let team = sqlx::query_as::<_, Team>(
        "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
         FROM teams WHERE id = ?",
    )
    .bind(&team_id)
    .fetch_one(pool)
    .await?;

    Ok(team)
}

/// Disband team. Pre-lock only.
pub async fn disband_team(pool: &SqlitePool, captain_id: &str) -> AppResult<()> {
    let mut tx = pool.begin().await?;

    let captain_membership = sqlx::query(
        "SELECT tm.team_id, t.is_locked
         FROM team_members tm
         JOIN teams t ON t.id = tm.team_id
         WHERE tm.user_id = ? AND tm.role = 'captain' AND t.disbanded = 0",
    )
    .bind(captain_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Forbidden("Only the captain can disband the team".into()))?;

    let team_id: String = captain_membership.try_get("team_id")?;
    let is_locked: bool = captain_membership.try_get("is_locked")?;

    if is_locked {
        return Err(AppError::Forbidden("Cannot disband a locked team".into()));
    }

    sqlx::query("UPDATE teams SET disbanded = 1 WHERE id = ?")
        .bind(&team_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM team_members WHERE team_id = ?")
        .bind(&team_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Lock all teams with at least TEAM_MIN_SIZE members (called at event start).
pub async fn lock_all_teams(pool: &SqlitePool) -> AppResult<u64> {
    let result = sqlx::query(
        "UPDATE teams
         SET is_locked = 1
         WHERE disbanded = 0 AND is_locked = 0
           AND (SELECT COUNT(*) FROM team_members WHERE team_id = teams.id) >= ?",
    )
    .bind(TEAM_MIN_SIZE as i64)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Check and auto-lock team if valid (>= TEAM_MIN_SIZE). Returns true if locked.
pub async fn lock_team_if_valid(pool: &SqlitePool, team_id: &str) -> AppResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM team_members WHERE team_id = ?",
    )
    .bind(team_id)
    .fetch_one(pool)
    .await?;

    if count as usize >= TEAM_MIN_SIZE {
        sqlx::query("UPDATE teams SET is_locked = 1 WHERE id = ? AND disbanded = 0")
            .bind(team_id)
            .execute(pool)
            .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Get team and member details for a user.
pub async fn get_team_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> AppResult<Option<TeamWithMembers>> {
    let team_id_row = sqlx::query("SELECT team_id FROM team_members WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;

    let Some(r) = team_id_row else { return Ok(None); };
    let team_id: String = r.try_get("team_id")?;

    let team = sqlx::query_as::<_, Team>(
        "SELECT id, name, join_code, captain_id, is_locked, disbanded, created_at
         FROM teams WHERE id = ? AND disbanded = 0",
    )
    .bind(&team_id)
    .fetch_optional(pool)
    .await?;

    let Some(team) = team else { return Ok(None); };

    let members = sqlx::query_as::<_, TeamMemberView>(
        "SELECT tm.user_id, u.username, u.display_name, tm.role, tm.joined_at
         FROM team_members tm
         JOIN users u ON u.id = tm.user_id
         WHERE tm.team_id = ?
         ORDER BY (CASE WHEN tm.role = 'captain' THEN 0 ELSE 1 END), tm.joined_at ASC",
    )
    .bind(&team.id)
    .fetch_all(pool)
    .await?;

    Ok(Some(TeamWithMembers { team, members }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;

    #[tokio::test]
    async fn test_team_lifecycle() {
        let (state, _dir) = test_support::state().await;
        let pool = &state.db;

        // Register two users
        let u1 = crate::services::auth::register(&state, "user_one", "User One", "Pass123456").await.unwrap();
        let u2 = crate::services::auth::register(&state, "user_two", "User Two", "Pass123456").await.unwrap();

        // User 1 creates team
        let team = create_team(pool, &u1, "Red Team", 30).await.unwrap();
        assert_eq!(team.name, "Red Team");
        assert_eq!(team.captain_id, u1);
        assert!(!team.is_locked);

        // Preview team
        let preview = preview_team_by_code(pool, &team.join_code).await.unwrap();
        assert_eq!(preview.members.len(), 1);
        assert_eq!(preview.members[0].role, "captain");

        // User 2 joins team
        let joined_team = join_team(pool, &u2, &team.join_code, 30).await.unwrap();
        assert_eq!(joined_team.id, team.id);

        let preview2 = preview_team_by_code(pool, &team.join_code).await.unwrap();
        assert_eq!(preview2.members.len(), 2);

        // Try adding a 3rd user -> full
        let u3 = crate::services::auth::register(&state, "user_three", "User Three", "Pass123456").await.unwrap();
        let err = join_team(pool, &u3, &team.join_code, 30).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));

        // Captain transfers captaincy to user 2
        transfer_captain(pool, &u1, &u2).await.unwrap();
        let updated = get_team_for_user(pool, &u2).await.unwrap().unwrap();
        assert_eq!(updated.team.captain_id, u2);

        // Rename team
        rename_team(pool, &u2, "Blue Team").await.unwrap();
        let renamed = get_team_for_user(pool, &u2).await.unwrap().unwrap();
        assert_eq!(renamed.team.name, "Blue Team");

        // Kick member (u2 kicks u1)
        kick_member(pool, &u2, &u1).await.unwrap();
        let preview3 = preview_team_by_code(pool, &team.join_code).await.unwrap();
        assert_eq!(preview3.members.len(), 1);

        // Disband team
        disband_team(pool, &u2).await.unwrap();
        assert!(get_team_for_user(pool, &u2).await.unwrap().is_none());
    }
}
