use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::{cmp::Ordering, collections::HashMap, time::Instant};

use crate::{errors::AppResult, state::AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreboardEntry {
    pub rank: usize,
    pub team_id: String,
    pub team_name: String,
    pub members: Vec<String>,
    pub total_score: i64,
    pub opportunity_points: i64,
    pub assigned_challenges: i64,
    pub solved_challenges: i64,
    pub performance_percent: f64,
    pub performance_label: String,
    pub hard_solved: i64,
    pub medium_solved: i64,
    pub easy_solved: i64,
    pub hard_solve_sum: i64,
    pub medium_solve_sum: i64,
    pub easy_solve_sum: i64,
    pub reached_score_at: Option<String>,
    pub reached_score_label: String,
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

    Ok(reached_from_events(events, final_score))
}

fn reached_from_events(mut events: Vec<(String, i64)>, final_score: i64) -> Option<String> {
    // No events → no timestamp. This is correct for teams with zero final score and
    // no activity; any team with a non-zero final score must have at least one event.
    if events.is_empty() {
        debug_assert_eq!(final_score, 0, "non-zero final_score with empty event history is a data inconsistency");
        return None;
    }

    // Chronological order (ISO-8601 / SQLite millisecond format sorts lexicographically)
    events.sort_by(|a, b| a.0.cmp(&b.0));

    let mut running = 0i64;
    let mut history = Vec::with_capacity(events.len());
    for (ts, delta) in events {
        running += delta;
        history.push((ts, running));
    }

    // Earliest event after which the running total equals final_score AND stays there.
    // Zero-point events are already excluded at the query level (points_awarded != 0
    // and delta != 0), so every entry in history represents a real score change.
    for i in 0..history.len() {
        if history[i..].iter().all(|(_, r)| *r == final_score) {
            return Some(history[i].0.clone());
        }
    }

    // Should be unreachable: the running total after all events must equal final_score
    // because final_score is computed from the same event rows. Guard against data skew.
    debug_assert!(false, "reached_from_events: final_score {final_score} was never matched in history");
    None
}

fn reached_time_cmp(a: &ScoreboardEntry, b: &ScoreboardEntry) -> Ordering {
    match (&a.reached_score_at, &b.reached_score_at) {
        (Some(ta), Some(tb)) => ta.cmp(tb),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn competitive_cmp(a: &ScoreboardEntry, b: &ScoreboardEntry) -> Ordering {
    b.total_score.cmp(&a.total_score)
        .then_with(|| reached_time_cmp(a, b))
}

/// Rank by awarded points, then the earliest time that score was reached.
/// Pool ratios are informational and cannot override the score/time ordering.
pub async fn compute_scoreboard(pool: &SqlitePool) -> AppResult<Vec<ScoreboardEntry>> {
    // One snapshot for totals, members and history. Batching also avoids hundreds
    // of pool acquisitions competing with player requests on a cold leaderboard.
    let mut tx = pool.begin().await?;
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
        unique_pool AS (
            SELECT team_id, challenge_id, MAX(opportunity_points) AS opportunity_points
            FROM team_challenge_pool
            GROUP BY team_id, challenge_id
        ),
        opportunity AS (
            SELECT
                p.team_id,
                COUNT(*) AS assigned_challenges,
                SUM(p.opportunity_points) AS opportunity_points
            FROM unique_pool p
            JOIN challenges c ON c.id = p.challenge_id
            WHERE c.is_active = 1 OR EXISTS (
                SELECT 1 FROM solves s
                WHERE s.team_id = p.team_id AND s.challenge_id = p.challenge_id
            )
            GROUP BY p.team_id
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
            COALESCE(o.opportunity_points, 0) AS opportunity_points,
            COALESCE(o.assigned_challenges, 0) AS assigned_challenges,
            COALESCE(a.hard_solved, 0)       AS hard_solved,
            COALESCE(a.medium_solved, 0)     AS medium_solved,
            COALESCE(a.easy_solved, 0)       AS easy_solved,
            COALESCE(a.hard_solve_sum, 0)    AS hard_solve_sum,
            COALESCE(a.medium_solve_sum, 0)  AS medium_solve_sum,
            COALESCE(a.easy_solve_sum, 0)    AS easy_solve_sum
        FROM teams t
        LEFT JOIN agg a   ON a.team_id = t.id
        LEFT JOIN adj     ON adj.team_id = t.id
        LEFT JOIN opportunity o ON o.team_id = t.id
        WHERE t.disbanded = 0 AND t.is_locked = 1",
    )
    .fetch_all(&mut *tx)
    .await?;

    let member_rows = sqlx::query(
        "SELECT tm.team_id, u.display_name FROM users u
         JOIN team_members tm ON tm.user_id = u.id
         ORDER BY tm.team_id, CASE WHEN tm.role = 'captain' THEN 0 ELSE 1 END, tm.joined_at ASC")
        .fetch_all(&mut *tx).await?;
    let event_rows = sqlx::query(
        "SELECT team_id, solved_at AS ts, points_awarded AS delta FROM solves WHERE points_awarded != 0
         UNION ALL SELECT team_id, created_at AS ts, delta FROM score_adjustments WHERE delta != 0")
        .fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let mut members_by_team: HashMap<String, Vec<String>> = HashMap::new();
    for row in member_rows {
        members_by_team.entry(row.try_get("team_id")?).or_default().push(row.try_get("display_name")?);
    }
    let mut events_by_team: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    for row in event_rows {
        events_by_team.entry(row.try_get("team_id")?).or_default().push((row.try_get("ts")?, row.try_get("delta")?));
    }
    let mut entries = Vec::with_capacity(rows.len());

    for r in rows {
        let team_id: String = r.try_get("team_id")?;
        let total_score: i64 = r.try_get("total_score")?;
        let opportunity_points: i64 = r.try_get("opportunity_points")?;
        let assigned_challenges: i64 = r.try_get("assigned_challenges")?;
        let hard_solved: i64 = r.try_get("hard_solved")?;
        let medium_solved: i64 = r.try_get("medium_solved")?;
        let easy_solved: i64 = r.try_get("easy_solved")?;
        let solved_challenges = hard_solved + medium_solved + easy_solved;
        let performance_percent = if opportunity_points > 0 {
            total_score as f64 * 100.0 / opportunity_points as f64
        } else {
            0.0
        };

        let members = members_by_team.remove(&team_id).unwrap_or_default();
        let reached_at = reached_from_events(events_by_team.remove(&team_id).unwrap_or_default(), total_score);

        entries.push(ScoreboardEntry {
            rank: 0,
            team_id,
            team_name: r.try_get("team_name")?,
            members,
            total_score,
            opportunity_points,
            assigned_challenges,
            solved_challenges,
            performance_percent,
            performance_label: format!("{performance_percent:.1}%"),
            hard_solved,
            medium_solved,
            easy_solved,
            hard_solve_sum: r.try_get("hard_solve_sum")?,
            medium_solve_sum: r.try_get("medium_solve_sum")?,
            easy_solve_sum: r.try_get("easy_solve_sum")?,
            reached_score_label: reached_at.as_deref().map(crate::templates::readable_ist_time).unwrap_or_else(|| "—".into()),
            reached_score_at: reached_at,
            team_created_at: r.try_get("team_created_at")?,
        });
    }

    // Team identity only stabilizes display order; it never breaks a competitive tie.
    entries.sort_by(|a, b| {
        competitive_cmp(a, b)
            .then_with(|| a.team_name.cmp(&b.team_name))
            .then_with(|| a.team_id.cmp(&b.team_id))
    });

    for idx in 0..entries.len() {
        entries[idx].rank = if idx > 0 && competitive_cmp(&entries[idx - 1], &entries[idx]) == Ordering::Equal {
            entries[idx - 1].rank
        } else {
            idx + 1
        };
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

    // One refresh at a time. Keep the cache lock through publication so an
    // invalidation cannot be lost between computing and storing an old result.
    let mut cache = state.scoreboard_cache.write().await;
    if let Some((cached_at, ref entries)) = *cache {
        if cached_at.elapsed() < ttl {
            return Ok(entries.clone());
        }
    }
    let fresh = compute_scoreboard(&state.db).await?;
    *cache = Some((Instant::now(), fresh.clone()));
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

    #[test]
    fn adjustment_history_preserves_reached_score_timestamp() {
        let events = vec![("t1".into(), 100), ("t3".into(), 10), ("t2".into(), -10)];
        assert_eq!(reached_from_events(events, 100), Some("t3".into()));
        assert_eq!(reached_from_events(vec![("t1".into(), 100)], 100), Some("t1".into()));
        assert_eq!(reached_from_events(vec![], 0), None);
    }

    #[tokio::test]
    async fn batched_scoreboard_matches_individual_histories_and_adjustments() {
        let (state, _dir) = crate::services::dynamic_scoring::tests::fixture(true, 2).await;
        crate::services::dynamic_scoring::submit(&state, "t0", "u0", "easy", "ferris{correct}").await.unwrap();
        crate::services::dynamic_scoring::submit(&state, "t1", "u1", "hard", "ferris{correct}").await.unwrap();
        sqlx::query("INSERT INTO score_adjustments(id,team_id,admin_id,delta,reason) VALUES
            ('up','t0','operator', 20, 'bonus'), ('down','t1','operator', -10, 'penalty')")
            .execute(&state.db).await.unwrap();
        let board = compute_scoreboard(&state.db).await.unwrap();
        assert_eq!(board.len(), 2);
        assert_eq!(board[0].team_id, "t1");
        assert_eq!(board[0].total_score, 690);
        assert_eq!(board[1].total_score, 120);
        for entry in board {
            assert_eq!(entry.reached_score_at, reached_score_at(&state.db, &entry.team_id, entry.total_score).await.unwrap());
            assert_eq!(entry.members.len(), 1);
        }
    }

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

        sqlx::query(
            "INSERT INTO team_challenge_pool
             (team_id, round_number, challenge_id, assigned_at, opportunity_points) VALUES
             (?, 1, 'c-hard', '2026-10-09 10:00:00.000', 100),
             (?, 1, 'c-easy', '2026-10-09 10:00:00.000', 100)",
        )
        .bind(&t1.id)
        .bind(&t2.id)
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
        // Equal points: the earlier final score wins.
        assert_eq!(sb[0].team_id, t2.id);
        assert_eq!(sb[0].rank, 1);
        assert_eq!(sb[1].team_id, t1.id);
        assert_eq!(sb[1].rank, 2);

        // Reproduce the production inversion: the earlier team has a larger pool
        // and lower completion. Neither metric may outrank its earlier score time.
        sqlx::query("INSERT INTO team_challenge_pool
            (team_id,round_number,challenge_id,opportunity_points) VALUES (?,1,'c-hard',100)")
            .bind(&t2.id).execute(pool).await.unwrap();
        let different_pools = compute_scoreboard(pool).await.unwrap();
        assert_eq!(different_pools[0].team_id, t2.id);
        assert_eq!(different_pools[0].performance_label, "50.0%");
        assert_eq!(different_pools[1].performance_label, "100.0%");
        // Different solve counts are also irrelevant at equal points and time.
        let mut fewer_solves = different_pools[0].clone();
        let mut more_solves = different_pools[1].clone();
        fewer_solves.total_score = 310;
        more_solves.total_score = 310;
        fewer_solves.solved_challenges = 5;
        more_solves.solved_challenges = 7;
        assert_eq!(competitive_cmp(&fewer_solves, &more_solves), Ordering::Less);
        more_solves.reached_score_at = fewer_solves.reached_score_at.clone();
        assert_eq!(competitive_cmp(&fewer_solves, &more_solves), Ordering::Equal);
        // A team with no score-changing activity sorts after a timed equal score.
        more_solves.reached_score_at = None;
        assert_eq!(competitive_cmp(&fewer_solves, &more_solves), Ordering::Less);

        sqlx::query("UPDATE solves SET solved_at = '2026-10-09 11:00:00.000' WHERE team_id = ?")
            .bind(&t1.id)
            .execute(pool)
            .await
            .unwrap();
        let tied = compute_scoreboard(pool).await.unwrap();
        assert_eq!(tied[0].rank, 1);
        assert_eq!(tied[1].rank, 1, "equal score and time should share rank despite different pool sizes");

        // Higher awarded points rank first, even when the performance ratio is lower.
        sqlx::query("UPDATE challenges SET points = 1000 WHERE id = 'c-easy'")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE team_challenge_pool SET opportunity_points = 1000
             WHERE team_id = ? AND challenge_id = 'c-easy'",
        )
        .bind(&t2.id)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE solves SET points_awarded = 500 WHERE team_id = ?")
            .bind(&t2.id)
            .execute(pool)
            .await
            .unwrap();
        let sb = compute_scoreboard(pool).await.unwrap();
        assert_eq!(sb[0].team_id, t2.id, "500 awarded points must beat 100 points");
        assert_eq!(sb[0].total_score, 500);
        assert_eq!(sb[1].total_score, 100);
    }
}
