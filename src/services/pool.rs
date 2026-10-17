use rand::seq::SliceRandom;
use sqlx::SqlitePool;

use crate::{
    config::DIFFICULTY_SPLIT,
    errors::AppResult,
    models::TeamChallengePool,
};

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
    repeat_penalty_points: i64,
) -> AppResult<Vec<TeamChallengePool>> {
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
        // Spec 6.1: repeat penalty is 10 * (earlier appearances of that challenge in this team's pools)
        let earlier_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM team_challenge_pool
             WHERE team_id = ? AND challenge_id = ? AND round_number < ?",
        )
        .bind(team_id)
        .bind(&cid)
        .bind(round_number)
        .fetch_one(&mut *tx)
        .await?;

        let penalty = earlier_count * repeat_penalty_points;

        sqlx::query(
            "INSERT INTO team_challenge_pool (team_id, round_number, challenge_id, repeat_penalty)
             VALUES (?, ?, ?, ?)",
        )
        .bind(team_id)
        .bind(round_number)
        .bind(&cid)
        .bind(penalty)
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

/// Assign 2 randomly-chosen selectives from the round's selective set to a single team.
/// Selectives always have repeat_penalty = 0 (they are admin-curated, not randomly recurring).
pub async fn assign_selectives_for_team(
    pool: &SqlitePool,
    team_id: &str,
    round_number: i64,
) -> AppResult<Vec<TeamChallengePool>> {
    // Fetch the selective challenge IDs chosen for this round
    let mut selective_ids = sqlx::query_scalar::<_, String>(
        "SELECT challenge_id FROM round_selectives WHERE round_number = ?",
    )
    .bind(round_number)
    .fetch_all(pool)
    .await?;

    if selective_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Shuffle and pick 2 (or fewer if less than 2 available)
    {
        let mut rng = rand::rngs::OsRng;
        selective_ids.shuffle(&mut rng);
    }
    let chosen: Vec<String> = selective_ids.into_iter().take(2).collect();

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

        sqlx::query(
            "INSERT INTO team_challenge_pool (team_id, round_number, challenge_id, repeat_penalty)
             VALUES (?, ?, ?, 0)",
        )
        .bind(team_id)
        .bind(round_number)
        .bind(&cid)
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
    }
}
