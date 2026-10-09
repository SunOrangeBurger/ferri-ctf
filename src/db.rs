use anyhow::Result;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::SqlitePool;
use std::{str::FromStr, time::Duration};

/// Connect, apply the four spec PRAGMAs on EVERY connection, and run migrations.
pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true);

    // SQLite has a single writer, so a big pool adds contention, not throughput (spec 13.2).
    let pool = SqlitePoolOptions::new()
        .max_connections(16)
        .connect_with(opts)
        .await?;

    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}
