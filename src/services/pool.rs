use rand::seq::SliceRandom;
use sqlx::SqlitePool;
use std::collections::HashSet;

use crate::{
    config::DIFFICULTY_SPLIT,
    errors::AppResult,
    models::TeamChallengePool,
};

async fn opportunity_points(pool: &SqlitePool, challenge_id: &str) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT CASE WHEN settings.enabled = 1
                     THEN rules.maximum_points ELSE c.points END
         FROM challenges c
         CROSS JOIN scoring_settings settings
         LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty = c.difficulty
         WHERE c.id = ?",
    )
    .bind(challenge_id)
    .fetch_one(pool)
    .await?)
}

/// Calculate difficulty quotas (easy, medium, hard) using largest-remainder method.
/// Ties go to the more common tier (easy > medium > hard).
pub fn difficulty_quotas(pool_size: usize, split: [u32; 3]) -> (usize, usize, usize) {
    if pool_size == 0 {
        return (0, 0, 0);
    }

    let e_exact = (pool_size as f64) * (split[0] as f64) / 100.0;
    let m_exact = (pool_size as f64) * (split[1] as f64) / 100.0;
    let h_exact = (pool_size as f64) * (split[2] as f64) / 100.0;

    let base_e = e_exact.floor() as usize;
    let base_m = m_exact.floor() as usize;
    let base_h = h_exact.floor() as usize;

    let allocated = base_e + base_m + base_h;
    let mut deficit = pool_size.saturating_sub(allocated);

    let mut rems = [
        (e_exact - base_e as f64, 0usize),
        (m_exact - base_m as f64, 1usize),
        (h_exact - base_h as f64, 2usize),
    ];

    // Sort by remainder descending; for ties, preserve tier order (0 < 1 < 2, i.e. easy > medium > hard)
    rems.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });

    let mut adds = [0usize; 3];
    for (_, tier_idx) in rems {
        if deficit > 0 {
            adds[tier_idx] += 1;
            deficit -= 1;
        }
    }

    (base_e + adds[0], base_m + adds[1], base_h + adds[2])
}

/// Assign challenges to a single team for the specified round.
/// Excludes solved challenges; samples randomly per tier; calculates and freezes repeat penalty.
pub async fn assign_round_pool(
    pool: &SqlitePool,
    team_id: &str,
    round_number: i64,
    pool_size: usize,
    _repeat_penalty_points: i64,
) -> AppResult<Vec<TeamChallengePool>> {
    if sqlx::query_scalar::<_, bool>("SELECT is_backlog_round FROM rounds WHERE round_number=?")
        .bind(round_number).fetch_one(pool).await? {
        return backlog_round_pool(pool, team_id, round_number).await;
    }
    if is_last_round(pool, round_number).await? {
        return last_round_pool(pool, team_id, round_number).await;
    }
    let selective_only: bool = sqlx::query_scalar(
        "SELECT is_selective_only FROM rounds WHERE round_number = ?",
    )
    .bind(round_number)
    .fetch_one(pool)
    .await?;
    if selective_only {
        return Ok(Vec::new());
    }

    let mut tx = pool.begin().await?;

    // Check if team already has pool assigned for this round
    let existing_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM team_challenge_pool WHERE team_id = ? AND round_number = ?",
    )
    .bind(team_id)
    .fetch_one(&mut *tx)
    .await?;

    if existing_count > 0 {
        let existing = sqlx::query_as::<_, TeamChallengePool>(
            "SELECT team_id, round_number, challenge_id, repeat_penalty, assigned_at
             FROM team_challenge_pool WHERE team_id = ? AND round_number = ?",
        )
        .bind(team_id)
        .bind(round_number)
        .fetch_all(&mut *tx)
        .await?;
        return Ok(existing);
    }

    let (quota_easy, quota_med, quota_hard) = difficulty_quotas(pool_size, DIFFICULTY_SPLIT);

    // Fetch active NON-selective challenges not solved by this team, grouped by difficulty
    let fetch_tier = |diff: &'static str| {
        let tid = team_id.to_string();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT id FROM challenges
                 WHERE is_active = 1 AND is_selective = 0 AND difficulty = ?
                   AND id NOT IN (SELECT challenge_id FROM solves WHERE team_id = ?)",
            )
            .bind(diff)
            .bind(&tid)
            .fetch_all(pool)
            .await
        }
    };

    let mut easy_ids = fetch_tier("easy").await?;
    let mut med_ids = fetch_tier("medium").await?;
    let mut hard_ids = fetch_tier("hard").await?;

    {
        let mut rng = rand::rngs::OsRng;
        easy_ids.shuffle(&mut rng);
        med_ids.shuffle(&mut rng);
        hard_ids.shuffle(&mut rng);
    }

    let chosen_easy = easy_ids.into_iter().take(quota_easy);
    let chosen_med = med_ids.into_iter().take(quota_med);
    let chosen_hard = hard_ids.into_iter().take(quota_hard);

    let all_chosen: Vec<String> = chosen_easy.chain(chosen_med).chain(chosen_hard).collect();

    let mut pool_entries = Vec::with_capacity(all_chosen.len());

    for cid in all_chosen {
        // Seen/repeat penalty removed
        let penalty = 0;
        let opportunity = opportunity_points(pool, &cid).await?;

        sqlx::query(
            "INSERT INTO team_challenge_pool
             (team_id, round_number, challenge_id, repeat_penalty, opportunity_points)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(team_id)
        .bind(round_number)
        .bind(&cid)
        .bind(penalty)
        .bind(opportunity)
        .execute(&mut *tx)
        .await?;

        pool_entries.push(TeamChallengePool {
            team_id: team_id.to_string(),
            round_number,
            challenge_id: cid,
            repeat_penalty: penalty,
            assigned_at: String::new(), // will be populated on fetch if needed
        });
    }

    tx.commit().await?;
    Ok(pool_entries)
}

/// Assign selectives from the round's selective set to a single team. Normal rounds get
/// two random choices; selectives-only rounds give every team the complete five-item set.
/// Selectives always have repeat_penalty = 0 (they are admin-curated, not randomly recurring).
pub async fn assign_selectives_for_team(
    pool: &SqlitePool,
    team_id: &str,
    round_number: i64,
) -> AppResult<Vec<TeamChallengePool>> {
    if sqlx::query_scalar::<_, bool>("SELECT is_backlog_round FROM rounds WHERE round_number=?")
        .bind(round_number).fetch_one(pool).await? {
        return backlog_round_pool(pool, team_id, round_number).await;
    }
    if is_last_round(pool, round_number).await? {
        return last_round_pool(pool, team_id, round_number).await;
    }
    // Fetch the selective challenge IDs chosen for this round
    let mut selective_ids = sqlx::query_scalar::<_, String>(
        "SELECT challenge_id FROM round_selectives WHERE round_number = ?",
    )
    .bind(round_number)
    .fetch_all(pool)
    .await?;

    // If none were manually selected for this round, automatically use active selectives (up to 5)
    // so selectives are uploaded/assigned regardless of who selects them (2 of 5 selection logic).
    if selective_ids.is_empty() {
        selective_ids = sqlx::query_scalar::<_, String>(
            "SELECT id FROM challenges
             WHERE is_active = 1 AND is_selective = 1
             ORDER BY created_at ASC LIMIT 5",
        )
        .fetch_all(pool)
        .await?;

        for sid in &selective_ids {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO round_selectives (round_number, challenge_id) VALUES (?, ?)",
            )
            .bind(round_number)
            .bind(sid)
            .execute(pool)
            .await;
        }
    }

    let solved: HashSet<String> = sqlx::query_scalar(
        "SELECT challenge_id FROM solves WHERE team_id = ?",
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    selective_ids.retain(|id| !solved.contains(id));

    if selective_ids.is_empty() {
        return Ok(Vec::new());
    }

    let selective_only: bool = sqlx::query_scalar(
        "SELECT is_selective_only FROM rounds WHERE round_number = ?",
    )
    .bind(round_number)
    .fetch_one(pool)
    .await?;

    // Normal rounds randomize the two assigned selectives. A special round preserves
    // the complete set so all teams see the same five challenges.
    if !selective_only {
        let mut rng = rand::rngs::OsRng;
        selective_ids.shuffle(&mut rng);
    }
    let chosen: Vec<String> = selective_ids
        .into_iter()
        .take(if selective_only { 5 } else { 2 })
        .collect();

    let mut entries = Vec::with_capacity(chosen.len());
    for cid in chosen {
        // Skip if this team already has this challenge in this round's pool
        // (e.g., if assign was called twice)
        let already: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM team_challenge_pool
             WHERE team_id = ? AND round_number = ? AND challenge_id = ?",
        )
        .bind(team_id)
        .bind(round_number)
        .bind(&cid)
        .fetch_one(pool)
        .await?;

        if already > 0 {
            continue;
        }

        let opportunity = opportunity_points(pool, &cid).await?;
        sqlx::query(
            "INSERT INTO team_challenge_pool
             (team_id, round_number, challenge_id, repeat_penalty, opportunity_points)
             VALUES (?, ?, ?, 0, ?)",
        )
        .bind(team_id)
        .bind(round_number)
        .bind(&cid)
        .bind(opportunity)
        .execute(pool)
        .await?;

        entries.push(TeamChallengePool {
            team_id: team_id.to_string(),
            round_number,
            challenge_id: cid,
            repeat_penalty: 0,
            assigned_at: String::new(),
        });
    }

    Ok(entries)
}

/// Assign challenge pools (standard + selectives) to all locked and active teams for a given round.
pub async fn assign_pools_for_round(
    pool: &SqlitePool,
    round_number: i64,
    pool_size: usize,
    repeat_penalty_points: i64,
) -> AppResult<usize> {
    let team_ids = sqlx::query_scalar::<_, String>(
        "SELECT id FROM teams WHERE is_locked = 1 AND disbanded = 0",
    )
    .fetch_all(pool)
    .await?;

    let mut count = 0;
    for tid in &team_ids {
        assign_round_pool(pool, tid, round_number, pool_size, repeat_penalty_points).await?;
        assign_selectives_for_team(pool, tid, round_number).await?;
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_difficulty_quotas() {
        // Pool size 15: 60/30/10 -> 9 easy, 5 med, 1 hard per spec
        assert_eq!(difficulty_quotas(15, [60, 30, 10]), (9, 5, 1));

        // Pool size 10: 60/30/10 -> 6 easy, 3 med, 1 hard
        assert_eq!(difficulty_quotas(10, [60, 30, 10]), (6, 3, 1));

        // Pool size 0
        assert_eq!(difficulty_quotas(0, [60, 30, 10]), (0, 0, 0));

        // Pool size 1
        assert_eq!(difficulty_quotas(1, [60, 30, 10]), (1, 0, 0));
    }

    #[tokio::test]
    async fn test_pool_excludes_selectives() {
        let (state, _dir) = crate::state::test_support::state().await;
        let u1 = crate::services::auth::register(&state, "user_one", "User One", "Pass123456").await.unwrap();
        let t1 = crate::services::team::create_team(&state.db, &u1, "Team Alpha", 30).await.unwrap();

        // Insert 1 normal easy challenge and 1 selective easy challenge
        sqlx::query(
            "INSERT INTO challenges (id, title, description, category, difficulty, points, flag_hmac, flag_salt, is_active, is_selective)
             VALUES ('normal-easy', 'Normal Easy', 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 0),
                    ('sel-easy', 'Selective Easy', 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 1)"
        )
        .execute(&state.db)
        .await
        .unwrap();

        // Insert round 1 for FK constraint
        sqlx::query("INSERT INTO rounds (round_number, started_at) VALUES (1, datetime('now'))")
            .execute(&state.db)
            .await
            .unwrap();

        // assign_round_pool with quota requiring easy challenges
        let entries = assign_round_pool(&state.db, &t1.id, 1, 5, 10).await.unwrap();
        let ids: Vec<&str> = entries.iter().map(|e| e.challenge_id.as_str()).collect();

        assert!(ids.contains(&"normal-easy"));
        assert!(!ids.contains(&"sel-easy"), "Selective challenges must never be included in standard assign_round_pool");

        sqlx::query(
            "INSERT INTO solves
             (team_id, challenge_id, user_id, round_number, points_awarded)
             VALUES (?, 'normal-easy', ?, 1, 100)",
        )
        .bind(&t1.id)
        .bind(&u1)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE rounds SET ended_at = datetime('now') WHERE round_number = 1")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO rounds (round_number, started_at) VALUES (2, datetime('now'))")
            .execute(&state.db)
            .await
            .unwrap();
        let next_round = assign_round_pool(&state.db, &t1.id, 2, 5, 10)
            .await
            .unwrap();
        assert!(
            next_round.iter().all(|e| e.challenge_id != "normal-easy"),
            "solved standard challenges must not reappear in later rounds"
        );
    }

    #[tokio::test]
    async fn selective_assignment_excludes_challenges_the_team_already_solved() {
        let (state, _dir) = crate::state::test_support::state().await;
        let user_id = crate::services::auth::register(
            &state,
            "selective_player",
            "Selective Player",
            "Pass123456",
        )
        .await
        .unwrap();
        let team = crate::services::team::create_team(
            &state.db,
            &user_id,
            "Selective Team",
            30,
        )
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO challenges
             (id, title, description, category, difficulty, points, flag_hmac, flag_salt, is_active, is_selective)
             VALUES ('sel-solved', 'Solved', 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 1),
                    ('sel-new', 'New', 'desc', 'Misc', 'easy', 100, 'hmac', 'salt', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO rounds (round_number, started_at, ended_at) VALUES
             (1, '2026-10-09 10:00:00.000', '2026-10-09 11:00:00.000'),
             (2, '2026-10-09 12:00:00.000', NULL)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO round_selectives (round_number, challenge_id) VALUES
             (2, 'sel-solved'), (2, 'sel-new')",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO solves
             (team_id, challenge_id, user_id, round_number, points_awarded, solved_at)
             VALUES (?, 'sel-solved', ?, 1, 100, '2026-10-09 10:30:00.000')",
        )
        .bind(&team.id)
        .bind(&user_id)
        .execute(&state.db)
        .await
        .unwrap();

        let assigned = assign_selectives_for_team(&state.db, &team.id, 2)
            .await
            .unwrap();
        assert_eq!(assigned.len(), 1);
        assert_eq!(assigned[0].challenge_id, "sel-new");
    }
}

async fn is_last_round(pool: &SqlitePool, round: i64) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT is_last_round FROM rounds WHERE round_number=?")
        .bind(round).fetch_one(pool).await?)
}

async fn last_round_pool(pool: &SqlitePool, team: &str, round: i64) -> AppResult<Vec<TeamChallengePool>> {
    sqlx::query("INSERT OR IGNORE INTO team_challenge_pool
        (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
        SELECT t.id,l.round_number,c.id,0,
          CASE WHEN settings.enabled=1 THEN rules.maximum_points ELSE c.points END
        FROM teams t CROSS JOIN last_round_challenges l JOIN challenges c ON c.id=l.challenge_id
        CROSS JOIN scoring_settings settings
        LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty=c.difficulty
        WHERE t.id=? AND t.is_locked=1 AND t.disbanded=0 AND l.round_number=? AND c.is_active=1")
        .bind(team).bind(round).execute(pool).await?;
    Ok(sqlx::query_as("SELECT team_id,round_number,challenge_id,repeat_penalty,assigned_at
        FROM team_challenge_pool WHERE team_id=? AND round_number=?")
        .bind(team).bind(round).fetch_all(pool).await?)
}

async fn backlog_round_pool(pool: &SqlitePool, team: &str, round: i64) -> AppResult<Vec<TeamChallengePool>> {
    sqlx::query("INSERT OR IGNORE INTO team_challenge_pool
        (team_id,round_number,challenge_id,repeat_penalty,opportunity_points)
        SELECT t.id,l.round_number,c.id,0,
          CASE WHEN settings.enabled=1 THEN rules.maximum_points ELSE c.points END
        FROM teams t CROSS JOIN backlog_round_challenges l JOIN challenges c ON c.id=l.challenge_id
        CROSS JOIN scoring_settings settings
        LEFT JOIN difficulty_scoring_rules rules ON rules.difficulty=c.difficulty
        WHERE t.id=? AND t.is_locked=1 AND t.disbanded=0 AND l.round_number=? AND c.is_active=1")
        .bind(team).bind(round).execute(pool).await?;
    Ok(sqlx::query_as("SELECT team_id,round_number,challenge_id,repeat_penalty,assigned_at
        FROM team_challenge_pool WHERE team_id=? AND round_number=?")
        .bind(team).bind(round).fetch_all(pool).await?)
}
