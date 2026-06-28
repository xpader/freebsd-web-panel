//! Entry point — CLI parsing, bootstrap, and server start.

mod app;
mod audit;
mod auth;
mod config;
mod db;
mod error;
mod handlers;
mod monitor;
mod state;
mod web_assets;

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::app::build;
use crate::state::AppState;

#[derive(Debug, Parser)]
#[command(name = "fwp", version, about = "FreeBSD Web Panel")]
struct Cli {
    /// Path to the configuration file.
    #[arg(short, long, default_value = "/usr/local/etc/fwp.toml")]
    config: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Load or create the config file.
    let config = config::Config::load_or_create(&cli.config)?;
    tracing::info!(listen = %config.server.listen, "configuration loaded");

    // Open the database.
    let db = db::open(&config.paths.db)?;
    tracing::info!(db = %config.paths.db.display(), "database ready");

    // Open the audit log (best-effort).
    let audit = match audit::AuditLog::open(&config.paths.audit) {
        Ok(log) => {
            tracing::info!(path = %config.paths.audit.display(), "audit log ready");
            Some(log)
        }
        Err(e) => {
            tracing::warn!(error = %e, "audit log unavailable; continuing without it");
            None
        }
    };

    let state = AppState {
        db,
        config: Arc::new(config.clone()),
        audit,
        web_root: Some(config.server.web_root.clone()),
    };

    let user_count = {
        let conn = state.db.lock().await;
        crate::db::user_count(&conn)?
    };
    if user_count == 0 {
        tracing::warn!("no users yet — first-run setup required via the web UI");
    }
    let app = build(state.clone());
    monitor::spawn_collector(state);

    // Parse listen address.
    let addr: SocketAddr = config.server.listen.parse().map_err(|e| {
        anyhow::anyhow!("invalid listen address '{}': {}", config.server.listen, e)
    })?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "FWP listening (HTTP)");

    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
