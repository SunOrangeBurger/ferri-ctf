//! Bait submissions are evidence of using a supplied token, not proof of AI use.
//! Penalties are ordinary auditable score adjustments, once per team per trap.
use sqlx::SqliteConnection;
use uuid::Uuid;
use crate::errors::{AppError, AppResult};

pub const IDS: [&str; 10] = [
    "archive-index-a17", "protocol-docket-b24",
    "ledger-audit-c31", "quiet-relay-d08",
    "cargo-manifest-e42", "clockwork-incident-f19",
    "missing-inventory-g27", "checksum-cabinet-h11",
    "mailroom-archive-j36", "backup-register-k05",
];

/// Queue once per team/trap. Retries never postpone the original due time.
pub async fn queue(conn: &mut SqliteConnection, team: &str, user: &str, id: &str, at: &str) -> AppResult<&'static str> {
    sqlx::query("INSERT OR IGNORE INTO honeypot_pending(team_id,challenge_id,user_id,submitted_at,due_at)
        VALUES (?,?,?,?,strftime('%Y-%m-%d %H:%M:%f',?,'+18 seconds'))")
        .bind(team).bind(id).bind(user).bind(at).bind(at).execute(&mut *conn).await?;
    let applied: Option<String> = sqlx::query_scalar("SELECT applied_at FROM honeypot_pending WHERE team_id=? AND challenge_id=?")
        .bind(team).bind(id).fetch_one(&mut *conn).await?;
    Ok(if applied.is_some() { "honeypot_triggered" } else { "pending_review" })
}

/// Durable processing also recovers overdue penalties after a server restart.
pub async fn process_due(state: &crate::state::AppState) -> AppResult<usize> {
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE event_state SET id=id WHERE id=1").execute(&mut *tx).await?;
    let at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%d %H:%M:%f','now')").fetch_one(&mut *tx).await?;
    let rows: Vec<(String,String,String)> = sqlx::query_as("SELECT team_id,user_id,challenge_id FROM honeypot_pending
        WHERE applied_at IS NULL AND due_at<=? ORDER BY due_at,team_id,challenge_id LIMIT 100")
        .bind(&at).fetch_all(&mut *tx).await?;
    for (team,user,id) in &rows {
        apply(&mut tx,team,user,id,&at).await?;
        sqlx::query("UPDATE honeypot_pending SET applied_at=? WHERE team_id=? AND challenge_id=?")
            .bind(&at).bind(team).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if !rows.is_empty() { super::scoring::invalidate_cache(state).await; }
    Ok(rows.len())
}

pub async fn apply(
    conn: &mut SqliteConnection, team: &str, user: &str, challenge: &str, at: &str,
) -> AppResult<bool> {
    let adjustment_id = format!("honeypot:{team}:{challenge}");
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM score_adjustments WHERE id=?)")
        .bind(&adjustment_id).fetch_one(&mut *conn).await?;
    if exists { return Ok(false); }
    let operator: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE is_ferris=1 ORDER BY created_at,id LIMIT 1")
        .fetch_optional(&mut *conn).await?;
    let operator = operator.ok_or_else(|| AppError::Conflict("Honeypot penalty requires an operator account".into()))?;
    let score: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT SUM(points_awarded) FROM solves WHERE team_id=?),0)
          + COALESCE((SELECT SUM(delta) FROM score_adjustments WHERE team_id=?),0)")
        .bind(team).bind(team).fetch_one(&mut *conn).await?;
    // Keep 80% of a positive score, rounding the remaining score down.
    // Never increase zero/negative scores.
    let delta = if score > 0 { score * 4 / 5 - score } else { 0 };
    let reason = format!("Honeypot token submitted by user {user}; challenge {challenge}; 20% score decrement: {score} -> {}. Token use is not proof of AI use.", score + delta);
    sqlx::query("INSERT INTO score_adjustments(id,team_id,admin_id,delta,reason,created_at) VALUES(?,?,?,?,?,?)")
        .bind(&adjustment_id).bind(team).bind(&operator).bind(delta).bind(&reason).bind(at)
        .execute(&mut *conn).await?;
    sqlx::query("INSERT INTO audit_log(id,admin_id,action,target_type,target_id,details,created_at) VALUES(?,?,'honeypot_triggered','team',?,?,?)")
        .bind(Uuid::new_v4().to_string()).bind(&operator).bind(team)
        .bind(serde_json::json!({"user_id":user,"challenge_id":challenge,"score_before":score,"score_after":score+delta,"delta":delta,"automated":true}).to_string())
        .bind(at).execute(&mut *conn).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{dynamic_scoring::{self, tests::fixture}, scoring};

    #[tokio::test]
    async fn last_round_includes_ten_unattempted_traps_and_delay_is_durable() {
        let (state, _dir) = fixture(false, 2).await;
        for id in IDS {
            sqlx::query("INSERT INTO challenges(id,title,description,category,difficulty,points,flag_hmac,flag_salt,is_active)
                SELECT ?,?,'','Misc','easy',100,flag_hmac,flag_salt,0 FROM challenges WHERE id='easy'")
                .bind(id).bind(id).execute(&state.db).await.unwrap();
        }
        sqlx::query("INSERT INTO solves(team_id,challenge_id,user_id,round_number,points_awarded)
            VALUES ('t0','easy','u0',1,201)").execute(&state.db).await.unwrap();
        crate::services::rounds::end_round(&state,1,"operator").await.unwrap();
        let round = crate::services::rounds::start_last_round(&state,None,"operator").await.unwrap();
        for team in ["t0","t1"] {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM team_challenge_pool WHERE team_id=? AND round_number=? AND challenge_id IN (SELECT id FROM challenges WHERE id IN ('archive-index-a17','protocol-docket-b24','ledger-audit-c31','quiet-relay-d08','cargo-manifest-e42','clockwork-incident-f19','missing-inventory-g27','checksum-cabinet-h11','mailroom-archive-j36','backup-register-k05'))")
                .bind(team).bind(round.round_number).fetch_one(&state.db).await.unwrap();
            assert_eq!(count,10);
        }
        assert_eq!(dynamic_scoring::submit(&state,"t0","u0",IDS[9],"ferris{correct}").await.unwrap(),"pending_review");
        let (due, elapsed): (String,i64) = sqlx::query_as("SELECT due_at,CAST(ROUND((julianday(due_at)-julianday(submitted_at))*86400) AS INTEGER) FROM honeypot_pending")
            .fetch_one(&state.db).await.unwrap();
        assert_eq!(elapsed,18);
        assert_eq!(process_due(&state).await.unwrap(),0);
        dynamic_scoring::submit(&state,"t0","u0",IDS[9],"ferris{correct}").await.unwrap();
        let retry_due: String = sqlx::query_scalar("SELECT due_at FROM honeypot_pending").fetch_one(&state.db).await.unwrap();
        assert_eq!(due,retry_due);
        // A fresh worker reads the persisted job, even after the round has ended.
        crate::services::rounds::end_round(&state,round.round_number,"operator").await.unwrap();
        sqlx::query("UPDATE honeypot_pending SET due_at='2000-01-01 00:00:00'").execute(&state.db).await.unwrap();
        assert_eq!(process_due(&state).await.unwrap(),1);
        assert_eq!(process_due(&state).await.unwrap(),0);
        let entries = scoring::compute_scoreboard(&state.db).await.unwrap();
        assert_eq!(entries.iter().find(|e| e.team_id=="t0").unwrap().total_score,160);
    }

    #[tokio::test]
    async fn bait_deducts_twenty_percent_once_per_team_and_trap_in_both_scoring_modes() {
        for enabled in [false, true] {
            let (state, _dir) = fixture(enabled, 2).await;
            for id in IDS {
                sqlx::query("INSERT INTO challenges(id,title,description,category,difficulty,points,flag_hmac,flag_salt)
                    SELECT ?,?,'','Misc','easy',0,flag_hmac,flag_salt FROM challenges WHERE id='easy'")
                    .bind(id).bind(id).execute(&state.db).await.unwrap();
                sqlx::query("INSERT INTO team_challenge_pool(team_id,round_number,challenge_id,opportunity_points)
                    VALUES ('t0',1,?,0)").bind(id).execute(&state.db).await.unwrap();
            }
            sqlx::query("INSERT INTO solves(team_id,challenge_id,user_id,round_number,points_awarded)
                VALUES ('t0','easy','u0',1,201)").execute(&state.db).await.unwrap();
            assert_eq!(dynamic_scoring::submit(&state,"t0","u0",IDS[0],"wrong").await.unwrap(),"incorrect");
            assert!(dynamic_scoring::submit(&state,"t1","u1",IDS[0],"ferris{correct}").await.is_err());
            assert!(dynamic_scoring::submit(&state,"t0","u1",IDS[0],"ferris{correct}").await.is_err());
            for (id, expected) in [(IDS[0],160),(IDS[0],160),(IDS[1],128)] {
                let status = dynamic_scoring::submit(&state,"t0","u0",id,"ferris{correct}").await.unwrap();
                assert!(status=="pending_review" || status=="honeypot_triggered");
                // Processing before the due time must not deduct anything.
                assert_eq!(process_due(&state).await.unwrap(),0);
                sqlx::query("UPDATE honeypot_pending SET due_at='2000-01-01 00:00:00' WHERE team_id='t0'")
                    .execute(&state.db).await.unwrap();
                process_due(&state).await.unwrap();
                let entries = scoring::compute_scoreboard(&state.db).await.unwrap();
                assert_eq!(entries.iter().find(|e| e.team_id=="t0").unwrap().total_score,expected);
            }
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM solves WHERE challenge_id IN ('archive-index-a17','protocol-docket-b24','ledger-audit-c31','quiet-relay-d08','cargo-manifest-e42','clockwork-incident-f19','missing-inventory-g27','checksum-cabinet-h11','mailroom-archive-j36','backup-register-k05')")
                .fetch_one(&state.db).await.unwrap();
            assert_eq!(count,0,"bait must never earn points or record a solve");
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action='honeypot_triggered'")
                .fetch_one(&state.db).await.unwrap();
            assert_eq!(count,2,"retry must not duplicate the penalty audit");
        }
    }

    #[tokio::test]
    async fn nonpositive_scores_are_not_increased_or_further_reduced() {
        let (state, _dir) = fixture(false, 2).await;
        sqlx::query("INSERT INTO score_adjustments(id,team_id,admin_id,delta,reason)
            VALUES ('negative','t1','operator',-5,'fixture')").execute(&state.db).await.unwrap();
        for team in ["t0","t1"] {
            let mut tx = state.db.begin().await.unwrap();
            apply(&mut tx,team,"u0",IDS[0],"2026-10-17 08:00:00.000").await.unwrap();
            tx.commit().await.unwrap();
        }
        let deltas: Vec<i64> = sqlx::query_scalar("SELECT delta FROM score_adjustments WHERE id LIKE 'honeypot:%'")
            .fetch_all(&state.db).await.unwrap();
        assert_eq!(deltas,vec![0,0]);
    }
}
