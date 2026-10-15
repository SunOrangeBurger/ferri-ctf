use super::*;
use crate::state::test_support;
use std::sync::Arc;

pub(crate) async fn fixture(enabled: bool, teams: usize) -> (Arc<AppState>, tempfile::TempDir) {
    let (state, dir) = test_support::state().await;
    sqlx::query("UPDATE scoring_settings SET enabled = ?")
        .bind(enabled)
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_admin, is_ferris)
        VALUES ('operator', 'operator', 'Operator', 'unused', 1, 1)",
    )
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO rounds (round_number, started_at, auto_end_at)
        VALUES (1, strftime('%Y-%m-%d %H:%M:%f', 'now'), strftime('%Y-%m-%d %H:%M:%f', 'now', '+6 hours'))")
        .execute(&state.db).await.unwrap();
    let salt = super::super::verifier::generate_salt();
    let hmac = super::super::verifier::hash_flag(&state.keys.flag, &salt, "ferris{correct}");
    for (id, difficulty) in [
        ("easy", "easy"),
        ("medium", "medium"),
        ("hard", "hard"),
        ("hidden", "easy"),
    ] {
        sqlx::query("INSERT INTO challenges (id, title, description, category, difficulty, points, flag_hmac, flag_salt)
            VALUES (?, ?, 'Description', 'Web', ?, 123, ?, ?)")
            .bind(id).bind(format!("Challenge {id}")).bind(difficulty).bind(&hmac).bind(&salt)
            .execute(&state.db).await.unwrap();
    }
    for i in 0..teams {
        let id = format!("u{i}");
        sqlx::query("INSERT INTO users (id, username, display_name, password_hash) VALUES (?, ?, ?, 'unused')")
            .bind(&id).bind(&id).bind(&id).execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO teams (id, name, join_code, captain_id, is_locked) VALUES (?, ?, ?, ?, 1)",
        )
        .bind(format!("t{i}"))
        .bind(format!("Team {i}"))
        .bind(format!("CODE{i:04}"))
        .bind(&id)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO team_members (team_id, user_id, role) VALUES (?, ?, 'captain')")
            .bind(format!("t{i}"))
            .bind(&id)
            .execute(&state.db)
            .await
            .unwrap();
        for challenge in ["easy", "medium", "hard"] {
            sqlx::query("INSERT INTO team_challenge_pool (team_id, round_number, challenge_id) VALUES (?, 1, ?)")
                .bind(format!("t{i}")).bind(challenge).execute(&state.db).await.unwrap();
        }
    }
    (state, dir)
}

fn easy() -> Rule {
    Rule {
        difficulty: "easy".into(),
        starting_points: 100,
        minimum_points: 40,
        maximum_points: 150,
        quiet_minutes: 15,
        recovery_per_minute: 1,
        drop_percent: 15,
    }
}

async fn award(state: &AppState, team: &str, challenge: &str) -> i64 {
    sqlx::query_scalar("SELECT points_awarded FROM solves WHERE team_id = ? AND challenge_id = ?")
        .bind(team)
        .bind(challenge)
        .fetch_one(&state.db)
        .await
        .unwrap()
}

#[test]
fn quiet_period_caps_drop_and_rounding() {
    let r = easy();
    let mut p = PriceState::initial("c", &r);
    assert_eq!(p.value(&r, 15 * 60_000 - 1), 100 * SCALE);
    assert_eq!(p.value(&r, 15 * 60_000), 100 * SCALE);
    assert_eq!(p.value(&r, 15 * 60_000 + 30_000), 100 * SCALE + SCALE / 2);
    assert_eq!(p.value(&r, 1000 * 60_000), 150 * SCALE);
    p.after_solve(&r, 0, 100 * SCALE);
    assert_eq!(p.value(&r, 0), 91 * SCALE);
    assert_eq!(p.value(&r, 24 * 60_000), 100 * SCALE);
    assert_eq!(
        p.value(&r, 1000 * 60_000),
        100 * SCALE,
        "cannot overtake previous gross award"
    );
    for _ in 0..1000 {
        let v = p.value(&r, 0);
        p.after_solve(&r, 0, v);
    }
    assert_eq!(p.value(&r, 0), 40 * SCALE);
    assert_eq!(p.value(&r, i64::MAX), 40 * SCALE);
}

#[test]
fn six_hour_simulation_30_to_45_challenges_50_teams() {
    // Deterministic event model: alternating active rounds and pauses; burst and
    // widely spaced solves. Different exposure starts exercise unreleased tasks.
    let profiles = [
        easy(),
        Rule {
            difficulty: "medium".into(),
            starting_points: 300,
            minimum_points: 175,
            maximum_points: 400,
            quiet_minutes: 20,
            recovery_per_minute: 2,
            drop_percent: 10,
        },
        Rule {
            difficulty: "hard".into(),
            starting_points: 700,
            minimum_points: 450,
            maximum_points: 1000,
            quiet_minutes: 30,
            recovery_per_minute: 3,
            drop_percent: 5,
        },
    ];
    for count in [30, 45] {
        let mut prices: Vec<_> = (0..count)
            .map(|i| PriceState::initial(&i.to_string(), &profiles[i % 3]))
            .collect();
        let mut previous = vec![i64::MAX; count];
        let mut solves = vec![0usize; count];
        let mut available_minutes = vec![0i64; count];
        let mut awards = Vec::new();
        for minute in 0..360 {
            let active = !matches!(minute, 80..=89 | 170..=179 | 260..=269);
            for i in 0..count {
                let rule = &profiles[i % 3];
                if active && minute >= (i % 4) * 20 {
                    available_minutes[i] += 1;
                }
                let value = prices[i].value(rule, available_minutes[i] * 60_000);
                assert!(
                    (rule.minimum_points * SCALE..=rule.maximum_points * SCALE).contains(&value)
                );
                if active && minute >= (i % 4) * 20 && minute % (2 + i % 7) == 0 && solves[i] < 50 {
                    assert!(value <= previous[i]);
                    previous[i] = value;
                    awards.push(value / SCALE);
                    prices[i].after_solve(rule, available_minutes[i] * 60_000, value);
                    solves[i] += 1;
                }
            }
        }
        assert!(solves.iter().all(|n| *n >= 30 && *n <= 50));
        assert!(awards.len() >= count * 30);
    }
}

#[tokio::test]
async fn concurrent_distinct_and_duplicate_solves_are_atomic() {
    let (state, _dir) = fixture(true, 50).await;
    let mut tasks = Vec::new();
    for i in 0..50 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            submit(
                &state,
                &format!("t{i}"),
                &format!("u{i}"),
                "easy",
                "ferris{correct}",
            )
            .await
            .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), "correct");
    }
    let got: Vec<i64> = sqlx::query_scalar(
        "SELECT points_awarded FROM solves WHERE challenge_id = 'easy' ORDER BY rowid",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    let mut expected = PriceState::initial("easy", &easy());
    for value in got {
        let current = expected.value(&easy(), 0);
        assert_eq!(value, current / SCALE);
        expected.after_solve(&easy(), 0, current);
    }
    let one = {
        let s = state.clone();
        tokio::spawn(async move {
            submit(&s, "t0", "u0", "medium", "ferris{correct}")
                .await
                .unwrap()
        })
    };
    let two = {
        let s = state.clone();
        tokio::spawn(async move {
            submit(&s, "t0", "u0", "medium", "ferris{correct}")
                .await
                .unwrap()
        })
    };
    let mut statuses = vec![one.await.unwrap(), two.await.unwrap()];
    statuses.sort();
    assert_eq!(statuses, ["already_solved", "correct"]);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM submissions WHERE challenge_id = 'medium'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn wrong_flags_deductions_zero_awards_and_static_mode() {
    for enabled in [false, true] {
        let (state, _dir) = fixture(enabled, 2).await;
        let mut conn = state.db.acquire().await.unwrap();
        let before = load_price(&mut conn, "easy", &easy()).await.unwrap();
        assert_eq!(
            submit(&state, "t0", "u0", "easy", "wrong").await.unwrap(),
            "incorrect"
        );
        let after = load_price(&mut conn, "easy", &easy()).await.unwrap();
        assert_eq!(before.anchor_units, after.anchor_units);
        assert_eq!(before.last_solve_active_ms, after.last_solve_active_ms);
        sqlx::query("UPDATE team_challenge_pool SET repeat_penalty = 10 WHERE team_id = 't0'")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO hints(id, challenge_id, content, point_cost) VALUES ('hint', 'easy', 'help', 20)")
            .execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO hint_usage(team_id, hint_id) VALUES ('t0', 'hint')")
            .execute(&state.db)
            .await
            .unwrap();
        submit(&state, "t0", "u0", "easy", "ferris{correct}")
            .await
            .unwrap();
        assert_eq!(
            award(&state, "t0", "easy").await,
            if enabled { 70 } else { 93 }
        );
        sqlx::query("UPDATE team_challenge_pool SET repeat_penalty = 1000 WHERE team_id = 't1'")
            .execute(&state.db)
            .await
            .unwrap();
        submit(&state, "t1", "u1", "easy", "ferris{correct}")
            .await
            .unwrap();
        assert_eq!(award(&state, "t1", "easy").await, 0);
        assert!(
            submit(&state, "t0", "u1", "hard", "ferris{correct}")
                .await
                .is_err(),
            "recheck current membership"
        );
    }
}

#[tokio::test]
async fn live_rules_checkpoint_old_growth_and_overrides_preserve_awards() {
    let (state, _dir) = fixture(true, 2).await;
    // Artificially expose the challenge for twenty minutes without sleeping.
    sqlx::query("UPDATE challenge_availability SET opened_at = strftime('%Y-%m-%d %H:%M:%f', 'now', '-20 minutes')
        WHERE challenge_id = 'easy'").execute(&state.db).await.unwrap();
    let mut changed = easy();
    changed.starting_points = 120;
    changed.recovery_per_minute = 10;
    update_rule(&state.db, &changed, 0, "operator", "Tune recovery")
        .await
        .unwrap();
    let mut conn = state.db.acquire().await.unwrap();
    let p = load_price(&mut conn, "easy", &changed).await.unwrap();
    assert!(
        (105 * SCALE..106 * SCALE).contains(&p.anchor_units),
        "old rate applies before save"
    );
    assert_eq!(
        p.value(&changed, p.anchor_active_ms + 60_000),
        p.anchor_units + 10 * SCALE
    );
    let hidden = load_price(&mut conn, "hidden", &changed).await.unwrap();
    assert_eq!(
        hidden.anchor_units,
        120 * SCALE,
        "new starting points only for uninitialized challenges"
    );
    submit(&state, "t0", "u0", "easy", "ferris{correct}")
        .await
        .unwrap();
    let earned = award(&state, "t0", "easy").await;
    override_points(&state.db, "easy", 140, "operator", "Correction")
        .await
        .unwrap();
    let p = load_price(&mut conn, "easy", &changed).await.unwrap();
    assert_eq!(p.anchor_units, 140 * SCALE);
    assert_eq!(p.ceiling_units, None);
    assert_eq!(p.anchor_active_ms, p.last_solve_active_ms);
    assert_eq!(award(&state, "t0", "easy").await, earned);
    assert!(
        override_points(&state.db, "easy", 151, "operator", "Invalid")
            .await
            .is_err()
    );
    assert!(override_points(&state.db, "easy", 100, "operator", " ")
        .await
        .is_err());
    assert_eq!(
        load_price(&mut conn, "easy", &changed)
            .await
            .unwrap()
            .anchor_units,
        140 * SCALE
    );
    assert!(
        update_rule(&state.db, &changed, 0, "operator", "Stale page")
            .await
            .is_err()
    );
    changed.starting_points = 80;
    changed.maximum_points = 90;
    update_rule(&state.db, &changed, 1, "operator", "Lower cap")
        .await
        .unwrap();
    assert_eq!(
        load_price(&mut conn, "easy", &changed)
            .await
            .unwrap()
            .anchor_units,
        90 * SCALE
    );
    let details: Vec<String> = sqlx::query_scalar("SELECT details FROM audit_log ORDER BY rowid")
        .fetch_all(&state.db)
        .await
        .unwrap();
    assert_eq!(details.len(), 3);
    for detail in details {
        let d: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert!(d.get("before").is_some() && d.get("after").is_some() && d.get("reason").is_some());
    }
    assert!(set_mode(&state.db, false, 2, "operator", "Switch")
        .await
        .is_err());
}

#[tokio::test]
async fn availability_pauses_at_round_boundaries_and_survives_restart() {
    let (state, _dir) = fixture(true, 1).await;
    sqlx::query("UPDATE rounds SET started_at = '2026-10-15 10:00:00.000', auto_end_at = '2026-10-15 11:00:00.000',
        ended_at = '2026-10-15 11:00:04.000' WHERE round_number = 1").execute(&state.db).await.unwrap();
    sqlx::query("UPDATE challenge_availability SET opened_at = '2026-10-15 10:00:00.000'")
        .execute(&state.db)
        .await
        .unwrap();
    let mut conn = state.db.acquire().await.unwrap();
    assert_eq!(
        active_clock(&mut conn, "easy", "2026-10-15 12:00:00.000")
            .await
            .unwrap(),
        60 * 60_000
    );
    sqlx::query("UPDATE rounds SET ended_at = '2026-10-15 10:45:00.000' WHERE round_number = 1")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO rounds(round_number, started_at, ended_at) VALUES (2, '2026-10-15 12:00:00.000', '2026-10-15 12:30:00.000')")
        .execute(&state.db).await.unwrap();
    sqlx::query(
        "INSERT INTO challenge_availability(challenge_id, round_number, opened_at, closed_at)
        VALUES ('easy', 2, '2026-10-15 12:10:00.000', '2026-10-15 12:20:00.000'),
               ('easy', 2, '2026-10-15 12:25:00.000', NULL)",
    )
    .execute(&state.db)
    .await
    .unwrap();
    assert_eq!(
        active_clock(&mut conn, "easy", "2026-10-15 14:00:00.000")
            .await
            .unwrap(),
        60 * 60_000
    );
    let reopened = crate::db::connect(&state.cfg.database_url).await.unwrap();
    let mut fresh = reopened.acquire().await.unwrap();
    assert_eq!(
        active_clock(&mut fresh, "easy", "2026-10-15 14:00:00.000")
            .await
            .unwrap(),
        60 * 60_000
    );
    assert_eq!(
        load_price(&mut fresh, "easy", &easy())
            .await
            .unwrap()
            .anchor_units,
        100 * SCALE
    );
}

#[tokio::test]
async fn activation_and_team_changes_track_single_exposure_windows() {
    let (state, _dir) = fixture(true, 2).await;
    let open = |state: Arc<AppState>| async move {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM challenge_availability WHERE challenge_id = 'easy' AND closed_at IS NULL")
            .fetch_one(&state.db).await.unwrap()
    };
    assert_eq!(
        open(state.clone()).await,
        1,
        "multiple pools share a window"
    );
    sqlx::query("UPDATE challenges SET is_active = 0")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(open(state.clone()).await, 0);
    sqlx::query("UPDATE challenges SET is_active = 1")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(open(state.clone()).await, 1);
    sqlx::query("UPDATE teams SET is_locked = 0")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(open(state.clone()).await, 0);
    sqlx::query("UPDATE teams SET is_locked = 1 WHERE id = 't0'")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(open(state.clone()).await, 1);
    sqlx::query("DELETE FROM team_challenge_pool WHERE challenge_id = 'easy'")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(open(state.clone()).await, 0);
}

#[test]
fn invalid_rules_are_rejected() {
    let mut r = easy();
    for value in [-1, 101, i64::MAX] {
        r.minimum_points = value;
        assert!(r.validate().is_err());
    }
    let mut r = easy();
    r.recovery_per_minute = i64::MAX;
    assert!(r.validate().is_err());
    let mut r = easy();
    r.drop_percent = 100;
    assert!(r.validate().is_err());
}

#[tokio::test]
async fn batched_quotes_match_individual_clocks_and_persisted_prices() {
    let (state, _dir) = fixture(true, 2).await;
    submit(&state, "t0", "u0", "easy", "ferris{correct}")
        .await
        .unwrap();
    override_points(&state.db, "hard", 800, "operator", "Quote parity")
        .await
        .unwrap();
    let mut tx = state.db.begin().await.unwrap();
    let at = now(&mut tx).await.unwrap();
    let rules = rules(&mut tx).await.unwrap();
    let quotes = prices(&mut tx, &at).await.unwrap();
    for quote in quotes {
        let rule = rules
            .iter()
            .find(|r| r.difficulty == quote.difficulty)
            .unwrap();
        let price = load_price(&mut tx, &quote.id, rule).await.unwrap();
        let clock = active_clock(&mut tx, &quote.id, &at).await.unwrap();
        assert_eq!(quote.current_points, price.value(rule, clock) / SCALE);
    }
}

#[tokio::test]
async fn concurrent_hint_unlocks_and_solves_never_grant_free_late_hints() {
    let (state, _dir) = fixture(true, 50).await;
    sqlx::query("INSERT INTO hints(id, challenge_id, content, point_cost) VALUES('race-hint', 'easy', 'hint', 10)")
        .execute(&state.db).await.unwrap();
    let mut tasks = Vec::new();
    for i in 0..50 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            let team = format!("t{i}");
            let user = format!("u{i}");
            let (hint, solve) = tokio::join!(
                crate::services::hints::request_hint(&state.db, &team, "race-hint"),
                submit(&state, &team, &user, "easy", "ferris{correct}")
            );
            assert_eq!(solve.unwrap(), "correct");
            (team, hint.is_ok())
        }));
    }
    let mut accepted = HashMap::new();
    for task in tasks {
        let (team, hint) = task.await.unwrap();
        accepted.insert(team, hint);
    }
    let rows = sqlx::query(
        "SELECT team_id, points_awarded FROM solves WHERE challenge_id = 'easy' ORDER BY rowid",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    let mut price = PriceState::initial("easy", &easy());
    for row in rows {
        let team: String = row.get("team_id");
        let gross = price.value(&easy(), 0);
        let expected = gross / SCALE - if accepted[&team] { 10 } else { 0 };
        assert_eq!(
            row.get::<i64, _>("points_awarded"),
            expected,
            "hint/solve order for {team}"
        );
        price.after_solve(&easy(), 0, gross);
    }
}

#[tokio::test]
async fn inactive_challenge_hints_are_not_available() {
    let (state, _dir) = fixture(true, 1).await;
    sqlx::query("INSERT INTO hints(id, challenge_id, content, point_cost) VALUES('inactive-hint', 'easy', 'hint', 10)")
        .execute(&state.db).await.unwrap();
    sqlx::query("UPDATE challenges SET is_active = 0 WHERE id = 'easy'")
        .execute(&state.db)
        .await
        .unwrap();
    assert!(matches!(
        crate::services::hints::request_hint(&state.db, "t0", "inactive-hint").await,
        Err(AppError::NotFound(_))
    ));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hint_usage")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn leaderboard_read_bursts_and_invalidations_preserve_completed_awards() {
    let (state, _dir) = fixture(true, 50).await;
    let mut tasks = Vec::new();
    for i in 0..50 {
        let solving_state = state.clone();
        tasks.push(tokio::spawn(async move {
            submit(
                &solving_state,
                &format!("t{i}"),
                &format!("u{i}"),
                "easy",
                "ferris{correct}",
            )
            .await
            .unwrap();
        }));
        let reading_state = state.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..3 {
                crate::services::scoring::get_scoreboard(&reading_state)
                    .await
                    .unwrap();
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let final_board = crate::services::scoring::get_scoreboard(&state)
        .await
        .unwrap();
    for entry in final_board {
        let total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(points_awarded), 0) FROM solves WHERE team_id = ?",
        )
        .bind(&entry.team_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(entry.total_score, total);
    }
}
