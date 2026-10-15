//! Shared challenge prices. All mutation callers acquire the SQLite writer before
//! reading, so solves, rule edits, and overrides have one database-defined order.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    state::AppState,
};

pub const SCALE: i64 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Rule {
    pub difficulty: String,
    pub starting_points: i64,
    pub minimum_points: i64,
    pub maximum_points: i64,
    pub quiet_minutes: i64,
    pub recovery_per_minute: i64,
    pub drop_percent: i64,
}

impl Rule {
    pub fn validate(&self) -> AppResult<()> {
        if !matches!(self.difficulty.as_str(), "easy" | "medium" | "hard")
            || self.minimum_points < 0
            || self.minimum_points > self.starting_points
            || self.starting_points > self.maximum_points
            || self.maximum_points > 1_000_000
            || !(1..=10080).contains(&self.quiet_minutes)
            || !(1..=1_000_000).contains(&self.recovery_per_minute)
            || !(1..=99).contains(&self.drop_percent)
        {
            return Err(AppError::BadRequest(
                "Invalid scoring rule: check bounds, positive rates, and drop percentage (1–99)"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Settings {
    pub enabled: bool,
    pub revision: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct PriceState {
    pub challenge_id: String,
    pub anchor_units: i64,
    pub anchor_active_ms: i64,
    pub last_solve_active_ms: i64,
    pub ceiling_units: Option<i64>,
}

impl PriceState {
    pub fn initial(id: &str, rule: &Rule) -> Self {
        Self {
            challenge_id: id.into(),
            anchor_units: rule.starting_points * SCALE,
            anchor_active_ms: 0,
            last_solve_active_ms: 0,
            ceiling_units: None,
        }
    }

    pub fn value(&self, rule: &Rule, active_ms: i64) -> i64 {
        let floor = rule.minimum_points * SCALE;
        let cap = self
            .ceiling_units
            .unwrap_or(rule.maximum_points * SCALE)
            .clamp(floor, rule.maximum_points * SCALE);
        let growth_start = self.anchor_active_ms.max(
            self.last_solve_active_ms
                .saturating_add(rule.quiet_minutes * 60_000),
        );
        let elapsed = active_ms.saturating_sub(growth_start).max(0);
        let growth =
            i128::from(rule.recovery_per_minute) * i128::from(SCALE) * i128::from(elapsed) / 60_000;
        (i128::from(self.anchor_units) + growth).clamp(i128::from(floor), i128::from(cap)) as i64
    }

    pub fn after_solve(&mut self, rule: &Rule, active_ms: i64, value: i64) {
        let floor = rule.minimum_points * SCALE;
        self.anchor_units = floor + (value - floor) * (100 - rule.drop_percent) / 100;
        self.anchor_active_ms = active_ms;
        self.last_solve_active_ms = active_ms;
        self.ceiling_units = Some(value);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChallengePrice {
    pub id: String,
    pub title: String,
    pub difficulty: String,
    pub current_points: i64,
    pub minimum_points: i64,
    pub maximum_points: i64,
}

pub async fn settings(conn: &mut SqliteConnection) -> AppResult<Settings> {
    Ok(
        sqlx::query_as("SELECT enabled, revision FROM scoring_settings WHERE id = 1")
            .fetch_one(conn)
            .await?,
    )
}

pub async fn rules(conn: &mut SqliteConnection) -> AppResult<Vec<Rule>> {
    Ok(sqlx::query_as(
        "SELECT * FROM difficulty_scoring_rules ORDER BY starting_points, difficulty",
    )
    .fetch_all(conn)
    .await?)
}

pub async fn now(conn: &mut SqliteConnection) -> AppResult<String> {
    Ok(
        sqlx::query_scalar("SELECT strftime('%Y-%m-%d %H:%M:%f', 'now')")
            .fetch_one(conn)
            .await?,
    )
}

/// The first statement takes the writer lock before any reads. Unlike a deferred
/// read-then-write transaction this cannot upgrade a stale WAL snapshot.
pub async fn write_transaction(pool: &SqlitePool) -> AppResult<Transaction<'_, Sqlite>> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE scoring_settings SET revision = revision WHERE id = 1")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

pub async fn active_clock(conn: &mut SqliteConnection, id: &str, at: &str) -> AppResult<i64> {
    // min() caps an interval at its closure, actual round end, scheduled deadline,
    // or request timestamp. Thus a delayed auto-end worker cannot award downtime.
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(MAX(0, CAST(ROUND((
            julianday(min(?, COALESCE(a.closed_at, ?), COALESCE(r.ended_at, ?), COALESCE(r.auto_end_at, ?)))
            - julianday(a.opened_at)) * 86400000) AS INTEGER))), 0)
         FROM challenge_availability a JOIN rounds r ON r.round_number = a.round_number
         WHERE a.challenge_id = ?")
        .bind(at).bind(at).bind(at).bind(at).bind(id).fetch_one(conn).await?)
}

pub async fn load_price(
    conn: &mut SqliteConnection,
    id: &str,
    rule: &Rule,
) -> AppResult<PriceState> {
    Ok(
        sqlx::query_as::<_, PriceState>("SELECT * FROM challenge_pricing WHERE challenge_id = ?")
            .bind(id)
            .fetch_optional(conn)
            .await?
            .unwrap_or_else(|| PriceState::initial(id, rule)),
    )
}

async fn save_price(conn: &mut SqliteConnection, price: &PriceState) -> AppResult<()> {
    sqlx::query("INSERT INTO challenge_pricing
        (challenge_id, anchor_units, anchor_active_ms, last_solve_active_ms, ceiling_units)
        VALUES (?, ?, ?, ?, ?) ON CONFLICT(challenge_id) DO UPDATE SET
        anchor_units = excluded.anchor_units, anchor_active_ms = excluded.anchor_active_ms,
        last_solve_active_ms = excluded.last_solve_active_ms, ceiling_units = excluded.ceiling_units")
        .bind(&price.challenge_id).bind(price.anchor_units).bind(price.anchor_active_ms)
        .bind(price.last_solve_active_ms).bind(price.ceiling_units).execute(conn).await?;
    Ok(())
}

pub async fn prices(conn: &mut SqliteConnection, at: &str) -> AppResult<Vec<ChallengePrice>> {
    let enabled = settings(conn).await?.enabled;
    let rule_map: HashMap<_, _> = rules(conn)
        .await?
        .into_iter()
        .map(|r| (r.difficulty.clone(), r))
        .collect();
    let rows = sqlx::query(
        "SELECT id, title, difficulty, points FROM challenges ORDER BY difficulty, title",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let difficulty: String = row.try_get("difficulty")?;
        let rule = &rule_map[&difficulty];
        let current_points = if enabled {
            let price = load_price(conn, &id, rule).await?;
            let clock = active_clock(conn, &id, at).await?;
            price.value(rule, clock) / SCALE
        } else {
            row.try_get("points")?
        };
        result.push(ChallengePrice {
            id,
            title: row.try_get("title")?,
            difficulty,
            current_points,
            minimum_points: rule.minimum_points,
            maximum_points: rule.maximum_points,
        });
    }
    Ok(result)
}

async fn audit(
    conn: &mut SqliteConnection,
    operator: &str,
    action: &str,
    target: &str,
    reason: &str,
    before: impl Serialize,
    after: impl Serialize,
) -> AppResult<()> {
    if reason.trim().is_empty() || reason.len() > 1000 {
        return Err(AppError::BadRequest(
            "A reason of 1–1000 characters is required".into(),
        ));
    }
    let details = serde_json::json!({ "reason": reason.trim(), "before": before, "after": after })
        .to_string();
    sqlx::query(
        "INSERT INTO audit_log (id, admin_id, action, target_type, target_id, details)
        VALUES (?, ?, ?, 'scoring', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(operator)
    .bind(action)
    .bind(target)
    .bind(details)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn set_mode(
    pool: &SqlitePool,
    enabled: bool,
    revision: i64,
    operator: &str,
    reason: &str,
) -> AppResult<()> {
    let mut tx = write_transaction(pool).await?;
    let before = settings(&mut tx).await?;
    if before.revision != revision {
        return Err(AppError::Conflict(
            "Settings changed; reload the page before saving".into(),
        ));
    }
    let started: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM rounds) OR
        EXISTS(SELECT 1 FROM event_state WHERE started_at IS NOT NULL)",
    )
    .fetch_one(&mut *tx)
    .await?;
    if started && before.enabled != enabled {
        return Err(AppError::Conflict("Select the scoring mode before starting the event; live rules and values remain editable".into()));
    }
    audit(
        &mut tx,
        operator,
        "scoring_mode",
        "event",
        reason,
        before.enabled,
        enabled,
    )
    .await?;
    sqlx::query("UPDATE scoring_settings SET enabled = ?, revision = revision + 1 WHERE id = 1")
        .bind(enabled)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn update_rule(
    pool: &SqlitePool,
    rule: &Rule,
    revision: i64,
    operator: &str,
    reason: &str,
) -> AppResult<()> {
    rule.validate()?;
    let mut tx = write_transaction(pool).await?;
    if settings(&mut tx).await?.revision != revision {
        return Err(AppError::Conflict(
            "Settings changed; reload the page before saving".into(),
        ));
    }
    let before = rules(&mut tx)
        .await?
        .into_iter()
        .find(|r| r.difficulty == rule.difficulty)
        .ok_or_else(|| AppError::BadRequest("Unknown difficulty".into()))?;
    let at = now(&mut tx).await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT p.challenge_id FROM challenge_pricing p
        JOIN challenges c ON c.id = p.challenge_id WHERE c.difficulty = ?",
    )
    .bind(&rule.difficulty)
    .fetch_all(&mut *tx)
    .await?;
    let mut checkpoints = Vec::new();
    for id in ids {
        let clock = active_clock(&mut tx, &id, &at).await?;
        let mut price = load_price(&mut tx, &id, &before).await?;
        let previous = price.value(&before, clock);
        price.anchor_units =
            previous.clamp(rule.minimum_points * SCALE, rule.maximum_points * SCALE);
        price.anchor_active_ms = clock;
        price.ceiling_units = price
            .ceiling_units
            .map(|c| c.clamp(rule.minimum_points * SCALE, rule.maximum_points * SCALE));
        checkpoints.push(serde_json::json!({"challenge_id": id, "before_units": previous, "after_units": price.anchor_units}));
        save_price(&mut tx, &price).await?;
    }
    audit(
        &mut tx,
        operator,
        "scoring_rule",
        &rule.difficulty,
        reason,
        &before,
        serde_json::json!({"rule": rule, "checkpoints": checkpoints}),
    )
    .await?;
    sqlx::query("UPDATE difficulty_scoring_rules SET starting_points = ?, minimum_points = ?,
        maximum_points = ?, quiet_minutes = ?, recovery_per_minute = ?, drop_percent = ? WHERE difficulty = ?")
        .bind(rule.starting_points).bind(rule.minimum_points).bind(rule.maximum_points)
        .bind(rule.quiet_minutes).bind(rule.recovery_per_minute).bind(rule.drop_percent).bind(&rule.difficulty)
        .execute(&mut *tx).await?;
    sqlx::query("UPDATE scoring_settings SET revision = revision + 1 WHERE id = 1")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn override_points(
    pool: &SqlitePool,
    id: &str,
    points: i64,
    operator: &str,
    reason: &str,
) -> AppResult<()> {
    let mut tx = write_transaction(pool).await?;
    if !settings(&mut tx).await?.enabled {
        return Err(AppError::Conflict(
            "Enable dynamic scoring before the event to override dynamic values".into(),
        ));
    }
    let difficulty: String = sqlx::query_scalar("SELECT difficulty FROM challenges WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("Challenge".into()))?;
    let rule = rules(&mut tx)
        .await?
        .into_iter()
        .find(|r| r.difficulty == difficulty)
        .unwrap();
    if !(rule.minimum_points..=rule.maximum_points).contains(&points) {
        return Err(AppError::BadRequest(
            "Current points must be within the difficulty's minimum and maximum".into(),
        ));
    }
    let at = now(&mut tx).await?;
    let clock = active_clock(&mut tx, id, &at).await?;
    let before = load_price(&mut tx, id, &rule).await?.value(&rule, clock);
    let price = PriceState {
        challenge_id: id.into(),
        anchor_units: points * SCALE,
        anchor_active_ms: clock,
        last_solve_active_ms: clock,
        ceiling_units: None,
    };
    audit(&mut tx, operator, "scoring_override", id, reason, before,
        serde_json::json!({"value_units": price.anchor_units, "recovery_ceiling": rule.maximum_points})).await?;
    save_price(&mut tx, &price).await?;
    tx.commit().await?;
    Ok(())
}

/// The only successful-submission path, used in both modes. A duplicate is an
/// idempotent result and never changes the market or logs another correct attempt.
pub async fn submit(
    state: &AppState,
    team: &str,
    user: &str,
    id: &str,
    flag: &str,
) -> AppResult<&'static str> {
    let mut tx = write_transaction(&state.db).await?;
    let at = now(&mut tx).await?;
    let row = sqlx::query(
        "SELECT c.points, c.difficulty, c.flag_hmac, c.flag_salt,
        p.repeat_penalty, r.round_number FROM team_challenge_pool p
        JOIN challenges c ON c.id = p.challenge_id JOIN teams t ON t.id = p.team_id
        JOIN rounds r ON r.round_number = p.round_number
        JOIN team_members tm ON tm.team_id = t.id AND tm.user_id = ?
        WHERE p.team_id = ? AND c.id = ? AND c.is_active = 1 AND t.is_locked = 1 AND t.disbanded = 0
          AND r.ended_at IS NULL AND (r.auto_end_at IS NULL OR r.auto_end_at > ?)
        ORDER BY r.round_number DESC LIMIT 1",
    )
    .bind(user)
    .bind(team)
    .bind(id)
    .bind(&at)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Challenge not in active round pool".into()))?;
    let solved: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM solves WHERE team_id = ? AND challenge_id = ?)",
    )
    .bind(team)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if solved {
        return Ok("already_solved");
    }
    let correct = super::verifier::verify_flag(
        &state.keys.flag,
        &row.try_get::<String, _>("flag_salt")?,
        flag,
        &row.try_get::<String, _>("flag_hmac")?,
    );
    sqlx::query("INSERT INTO submissions (id, team_id, user_id, challenge_id, is_correct, submitted_hash, submitted_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(team).bind(user).bind(id).bind(correct)
        .bind(super::verifier::hash_submission(&state.keys.flag, flag)).bind(&at).execute(&mut *tx).await?;
    if !correct {
        tx.commit().await?;
        return Ok("incorrect");
    }
    let mut gross: i64 = row.try_get("points")?;
    if settings(&mut tx).await?.enabled {
        let difficulty: String = row.try_get("difficulty")?;
        let rule = rules(&mut tx)
            .await?
            .into_iter()
            .find(|r| r.difficulty == difficulty)
            .unwrap();
        let clock = active_clock(&mut tx, id, &at).await?;
        let mut price = load_price(&mut tx, id, &rule).await?;
        let value = price.value(&rule, clock);
        gross = value / SCALE;
        price.after_solve(&rule, clock, value);
        save_price(&mut tx, &price).await?;
    }
    let hint_cost: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(h.point_cost), 0) FROM hint_usage hu
        JOIN hints h ON h.id = hu.hint_id WHERE hu.team_id = ? AND h.challenge_id = ?",
    )
    .bind(team)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let penalty: i64 = row.try_get("repeat_penalty")?;
    let awarded = gross
        .saturating_sub(penalty)
        .saturating_sub(hint_cost)
        .max(0);
    sqlx::query("INSERT INTO solves (team_id, challenge_id, user_id, round_number, points_awarded, solved_at)
        VALUES (?, ?, ?, ?, ?, ?)").bind(team).bind(id).bind(user)
        .bind(row.try_get::<i64, _>("round_number")?).bind(awarded).bind(&at).execute(&mut *tx).await?;
    tx.commit().await?;
    super::scoring::invalidate_cache(state).await;
    Ok("correct")
}

#[cfg(test)]
pub(crate) mod tests;
