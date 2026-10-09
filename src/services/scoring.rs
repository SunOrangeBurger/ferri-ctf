use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::time::Instant;

use crate::{errors::AppResult, state::AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreboardEntry {
    pub rank: usize,
    pub team_id: String,
    pub team_name: String,
    pub members: Vec<String>,
    pub total_score: i64,
    pub hard_solved: i64,
    pub medium_solved: i64,
    pub easy_solved: i64,
    pub hard_solve_sum: i64,
    pub medium_solve_sum: i64,
    pub easy_solve_sum: i64,
    pub reached_score_at: Option<String>,
    pub team_created_at: String,
}

/// Computes the timestamp at which the team first reached its final score and stayed there (spec 7.3).
pub async fn reached_score_at(
    pool: &SqlitePool,
    team_id: &str,
    final_score: i64,
) -> AppResult<Option<String>> {
    // Collect solves (timestamp, points_awarded)
    let solve_events = sqlx::query(
        "SELECT solved_at AS ts, points_awarded AS delta
         FROM solves
         WHERE team_id = ? AND points_awarded != 0",
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?;

    // Collect score adjustments (timestamp, delta)
    let adj_events = sqlx::query(
        "SELECT created_at AS ts, delta
         FROM score_adjustments
         WHERE team_id = ? AND delta != 0",
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?;

    let mut events: Vec<(String, i64)> = Vec::with_capacity(solve_events.len() + adj_events.len());

    for r in solve_events {
        events.push((r.try_get("ts")?, r.try_get("delta")?));
    }
    for r in adj_events {
        events.push((r.try_get("ts")?, r.try_get("delta")?));
    }

    if events.is_empty() {
        return Ok(None);
    }

    // Chronological order
    events.sort_by(|a, b| a.0.cmp(&b.0));

    let mut running = 0i64;
    let mut history = Vec::with_capacity(events.len());
    for (ts, delta) in events {
        running += delta;
        history.push((ts, running));
    }

    // Earliest event after which the running total equals final_score and never changes again
    for i in 0..history.len() {
        if history[i..].iter().all(|(_, r)| *r == final_score) {
            return Ok(Some(history[i].0.clone()));
        }
    }

    Ok(None)
}

/// Compute full scoreboard ranking per spec 7.1 - 7.3.
pub async fn compute_scoreboard(pool: &SqlitePool) -> AppResult<Vec<ScoreboardEntry>> {
    let rows = sqlx::query(
        "WITH solve_counts AS (
            SELECT challenge_id, COUNT(*) AS n_solves
            FROM solves
            GROUP BY challenge_id
        ),
        adj AS (
            SELECT team_id, SUM(delta) AS adj_total
            FROM score_adjustments
            GROUP BY team_id
        ),
        agg AS (
            SELECT
                s.team_id,
                SUM(s.points_awarded) AS solve_points,
                SUM(CASE WHEN c.difficulty = 'hard'   THEN 1 ELSE 0 END) AS hard_solved,
                SUM(CASE WHEN c.difficulty = 'medium' THEN 1 ELSE 0 END) AS medium_solved,
                SUM(CASE WHEN c.difficulty = 'easy'   THEN 1 ELSE 0 END) AS easy_solved,
                SUM(CASE WHEN c.difficulty = 'hard'   THEN sc.n_solves ELSE 0 END) AS hard_solve_sum,
                SUM(CASE WHEN c.difficulty = 'medium' THEN sc.n_solves ELSE 0 END) AS medium_solve_sum,
                SUM(CASE WHEN c.difficulty = 'easy'   THEN sc.n_solves ELSE 0 END) AS easy_solve_sum
            FROM solves s
            JOIN challenges c    ON c.id = s.challenge_id
            JOIN solve_counts sc ON sc.challenge_id = s.challenge_id
            GROUP BY s.team_id
        )
        SELECT
            t.id AS team_id,
            t.name AS team_name,
            t.created_at AS team_created_at,
            COALESCE(a.solve_points, 0) + COALESCE(adj.adj_total, 0) AS total_score,
            COALESCE(a.hard_solved, 0)       AS hard_solved,
            COALESCE(a.medium_solved, 0)     AS medium_solved,
            COALESCE(a.easy_solved, 0)       AS easy_solved,
            COALESCE(a.hard_solve_sum, 0)    AS hard_solve_sum,
            COALESCE(a.medium_solve_sum, 0)  AS medium_solve_sum,
            COALESCE(a.easy_solve_sum, 0)    AS easy_solve_sum
        FROM teams t
        LEFT JOIN agg a   ON a.team_id = t.id
        LEFT JOIN adj     ON adj.team_id = t.id
        WHERE t.disbanded = 0 AND t.is_locked = 1",
    )
    .fetch_all(pool)
    .await?;

    let mut entries = Vec::with_capacity(rows.len());

    for r in rows {
        let team_id: String = r.try_get("team_id")?;
        let total_score: i64 = r.try_get("total_score")?;

        let members: Vec<String> = sqlx::query_scalar(
            "SELECT u.display_name
             FROM users u
             JOIN team_members tm ON tm.user_id = u.id
             WHERE tm.team_id = ?
             ORDER BY (CASE WHEN tm.role = 'captain' THEN 0 ELSE 1 END), tm.joined_at ASC",
        )
        .bind(&team_id)
        .fetch_all(pool)
        .await?;

        let reached_at = reached_score_at(pool, &team_id, total_score).await?;

        entries.push(ScoreboardEntry {
            rank: 0,
            team_id,
            team_name: r.try_get("team_name")?,
            members,
            total_score,
            hard_solved: r.try_get("hard_solved")?,
            medium_solved: r.try_get("medium_solved")?,
            easy_solved: r.try_get("easy_solved")?,
            hard_solve_sum: r.try_get("hard_solve_sum")?,
            medium_solve_sum: r.try_get("medium_solve_sum")?,
            easy_solve_sum: r.try_get("easy_solve_sum")?,
            reached_score_at: reached_at,
            team_created_at: r.try_get("team_created_at")?,
        });
    }

    // Rust sorting for time tie-break and complete rank order
    entries.sort_by(|a, b| {
        b.total_score
            .cmp(&a.total_score)
            .then_with(|| b.hard_solved.cmp(&a.hard_solved))
            .then_with(|| b.medium_solved.cmp(&a.medium_solved))
            .then_with(|| b.easy_solved.cmp(&a.easy_solved))
            .then_with(|| a.hard_solve_sum.cmp(&b.hard_solve_sum))
            .then_with(|| a.medium_solve_sum.cmp(&b.medium_solve_sum))
            .then_with(|| a.easy_solve_sum.cmp(&b.easy_solve_sum))
            .then_with(|| match (&a.reached_score_at, &b.reached_score_at) {
                (Some(ta), Some(tb)) => ta.cmp(tb),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.team_created_at.cmp(&b.team_created_at))
    });

    for (idx, entry) in entries.iter_mut().enumerate() {
        entry.rank = idx + 1;
    }

    Ok(entries)
}

/// Retrieve scoreboard, using in-memory cache if fresh.
pub async fn get_scoreboard(state: &AppState) -> AppResult<Vec<ScoreboardEntry>> {
    let ttl = std::time::Duration::from_secs(state.cfg.scoreboard_cache_ttl_seconds);

    {
        let cache = state.scoreboard_cache.read().await;
        if let Some((cached_at, ref entries)) = *cache {
            if cached_at.elapsed() < ttl {
                return Ok(entries.clone());
            }
        }
    }

    let fresh = compute_scoreboard(&state.db).await?;
    {
        let mut cache = state.scoreboard_cache.write().await;
        *cache = Some((Instant::now(), fresh.clone()));
    }
    Ok(fresh)
}

/// Invalidate in-memory scoreboard cache.
pub async fn invalidate_cache(state: &AppState) {
    let mut cache = state.scoreboard_cache.write().await;
    *cache = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;

    #[tokio::test]
    async fn test_scoring_and_tie_breaks() {
        let (state, _dir) = test_support::state().await;
        let pool = &state.db;

        // Create two teams and locked
        let u1 = crate::services::auth::register(&state, "u1_cap", "User 1", "Pass123456").await.unwrap();
        let u2 = crate::services::auth::register(&state, "u2_cap", "User 2", "Pass123456").await.unwrap();

        let t1 = crate::services::team::create_team(pool, &u1, "Team One", 30).await.unwrap();
        let t2 = crate::services::team::create_team(pool, &u2, "Team Two", 30).await.unwrap();

        // Lock both teams
        sqlx::query("UPDATE teams SET is_locked = 1 WHERE id IN (?, ?)")
            .bind(&t1.id)
            .bind(&t2.id)
            .execute(pool)
            .await
            .unwrap();

        // Insert a round
        sqlx::query("INSERT INTO rounds (round_number, started_at) VALUES (1, datetime('now'))")
            .execute(pool)
            .await
            .unwrap();

        // Insert challenges: one hard (100 pts), one easy (100 pts)
        sqlx::query(
            "INSERT INTO challenges (id, title, description, category, difficulty, points, flag_hmac, flag_salt)
             VALUES ('c-hard', 'Hard Chal', 'desc', 'Web', 'hard', 100, 'hmac', 'salt'),
                    ('c-easy', 'Easy Chal', 'desc', 'Web', 'easy', 100, 'hmac', 'salt')"
        )
        .execute(pool)
        .await
        .unwrap();

        // t1 solves hard challenge (100 pts)
        sqlx::query(
            "INSERT INTO solves (team_id, challenge_id, user_id, round_number, points_awarded, solved_at)
             VALUES (?, 'c-hard', ?, 1, 100, '2026-10-09 12:00:00.000')"
        )
        .bind(&t1.id)
        .bind(&u1)
        .execute(pool)
        .await
        .unwrap();

        // t2 solves easy challenge (100 pts)
        sqlx::query(
            "INSERT INTO solves (team_id, challenge_id, user_id, round_number, points_awarded, solved_at)
             VALUES (?, 'c-easy', ?, 1, 100, '2026-10-09 11:00:00.000')"
        )
        .bind(&t2.id)
        .bind(&u2)
        .execute(pool)
        .await
        .unwrap();

        let sb = compute_scoreboard(pool).await.unwrap();
        assert_eq!(sb.len(), 2);
        // Both have 100 points, but t1 has 1 hard solved vs t2 0 hard solved!
        // Hard countback puts t1 at rank 1!
        assert_eq!(sb[0].team_id, t1.id);
        assert_eq!(sb[0].rank, 1);
        assert_eq!(sb[1].team_id, t2.id);
        assert_eq!(sb[1].rank, 2);
    }
}
