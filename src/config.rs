use anyhow::{bail, Result};
use std::{env, fmt, str::FromStr};

const MIN_SECRET_BYTES: usize = 32;

/// Easy/medium/hard share of each pool, in percent (spec 3). Fixed, not env-overridable.
pub const DIFFICULTY_SPLIT: [u32; 3] = [60, 30, 10];

pub const TEAM_MIN_SIZE: usize = 1;
pub const TEAM_MAX_SIZE: usize = 2;
pub const JOIN_CODE_LENGTH: usize = 8;

#[derive(Clone)]
pub struct Config {
    pub server_secret: Vec<u8>,
    /// Only needed on first boot (bootstrap). Checked at bootstrap time, not here.
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub database_url: String,
    pub bind_addr: String,
    pub cookie_secure: bool,

    pub planned_questions: u32,
    pub round_count: u32,
    pub pool_size_per_team: u32,
    pub repeat_penalty_points: i64,
    pub grace_period_minutes: i64,
    pub max_upload_bytes: usize,
    pub session_ttl_hours: i64,
    pub scoreboard_cache_ttl_seconds: u64,
}

// Manual Debug so secrets can never leak through `{:?}` or tracing.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("server_secret", &"<redacted>")
            .field("database_url", &self.database_url)
            .field("bind_addr", &self.bind_addr)
            .field("cookie_secure", &self.cookie_secure)
            .field("pool_size_per_team", &self.pool_size_per_team)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let secret = env::var("SERVER_SECRET").unwrap_or_default();
        if secret.len() < MIN_SECRET_BYTES {
            bail!(
                "SERVER_SECRET must be at least {MIN_SECRET_BYTES} bytes (got {}). \
                 Generate one with: openssl rand -hex 32",
                secret.len()
            );
        }

        let planned_questions: u32 = env_parse("PLANNED_QUESTIONS", 60)?;
        let round_count: u32 = env_parse("ROUND_COUNT", 4)?;
        if round_count == 0 {
            bail!("ROUND_COUNT must be at least 1");
        }
        // Spec 3: PLANNED_QUESTIONS / ROUND_COUNT rounded to nearest, unless overridden.
        let derived_pool = (planned_questions + round_count / 2) / round_count;
        let pool_size_per_team: u32 = env_parse("POOL_SIZE_PER_TEAM", derived_pool)?;
        if pool_size_per_team == 0 {
            bail!("POOL_SIZE_PER_TEAM must be at least 1");
        }

        Ok(Self {
            server_secret: secret.into_bytes(),
            admin_username: env_opt("ADMIN_USERNAME"),
            admin_password: env_opt("ADMIN_PASSWORD"),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://ferrisctf.db?mode=rwc".to_string()),
            bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            cookie_secure: env_bool("COOKIE_SECURE", false)?,
            planned_questions,
            round_count,
            pool_size_per_team,
            repeat_penalty_points: env_parse("REPEAT_PENALTY_POINTS", 10)?,
            grace_period_minutes: env_parse("GRACE_PERIOD_MINUTES", 30)?,
            max_upload_bytes: env_parse("MAX_UPLOAD_BYTES", 31_457_280)?,
            session_ttl_hours: env_parse("SESSION_TTL_HOURS", 24)?,
            scoreboard_cache_ttl_seconds: env_parse("SCOREBOARD_CACHE_TTL_SECONDS", 30)?,
        })
    }
}

fn env_opt(key: &str) -> Option<String> {
    env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    match env_opt(key) {
        None => Ok(default),
        Some(v) => v
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("{key}: invalid value '{v}': {e}")),
    }
}

fn env_bool(key: &str, default: bool) -> Result<bool> {
    match env_opt(key) {
        None => Ok(default),
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => bail!("{key}: expected true/false, got '{other}'"),
        },
    }
}
