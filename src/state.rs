use crate::{
    config::Config, keys::Keys, passwords::Passwords, ratelimit::RateLimiter,
    templates::Templates,
};
use sqlx::SqlitePool;
use std::{sync::Arc, time::Instant};

pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Config,
    pub keys: Keys,
    pub passwords: Passwords,
    pub limiter: RateLimiter,
    pub templates: Templates,
    pub started: Instant,
}

impl AppState {
    pub fn new(db: SqlitePool, cfg: Config) -> Arc<Self> {
        let keys = Keys::derive(&cfg.server_secret);
        Arc::new(Self {
            db,
            cfg,
            keys,
            passwords: Passwords::new(),
            limiter: RateLimiter::new(),
            templates: Templates::new(),
            started: Instant::now(),
        })
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    pub async fn state() -> (Arc<AppState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("t.db").display());
        let db = crate::db::connect(&url).await.unwrap();
        let cfg = Config {
            server_secret: vec![7u8; 32],
            admin_username: None,
            admin_password: None,
            database_url: url,
            bind_addr: "127.0.0.1:0".into(),
            cookie_secure: false,
            planned_questions: 60,
            round_count: 4,
            pool_size_per_team: 15,
            repeat_penalty_points: 10,
            grace_period_minutes: 30,
            max_upload_bytes: 31_457_280,
            session_ttl_hours: 24,
            scoreboard_cache_ttl_seconds: 30,
        };
        (AppState::new(db, cfg), dir)
    }
}
