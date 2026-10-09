#![allow(dead_code)] // temporary: remove once routes use everything

mod config;
mod csrf;
mod db;
mod errors;
mod keys;
mod middleware;
mod passwords;
mod routes;
mod services;
mod session;
mod state;

use std::{net::SocketAddr, time::Duration};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = config::Config::from_env()?;
    tracing::info!(?cfg, "config loaded");

    let pool = db::connect(&cfg.database_url).await?;
    let bind_addr = cfg.bind_addr.clone();
    let state = state::AppState::new(pool, cfg);

    // Fails fast (before listening) if there is no admin and no usable bootstrap env.
    services::auth::bootstrap_ferris(&state).await?;

    // Housekeeping: drop expired sessions every 10 minutes.
    tokio::spawn({
        let pool = state.db.clone();
        async move {
            let mut tick = tokio::time::interval(Duration::from_secs(600));
            loop {
                tick.tick().await;
                match session::purge_expired(&pool).await {
                    Ok(n) if n > 0 => tracing::debug!(purged = n, "expired sessions removed"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "session purge failed"),
                }
            }
        }
    });

    let app = middleware::secure(routes::router(), state.clone());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!(%bind_addr, "listening");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("shut down cleanly");
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutdown signal received");
}
