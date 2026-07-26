//! Entry point — CLI parsing, bootstrap, and server start.

mod app;
mod audit;
mod auth;
mod bgtask;
mod bhyve;
mod cmd;
mod config;
mod db;
mod error;
mod firewall_gen;
mod handlers;
mod ifutil;
mod jail;
mod monitor;
mod state;
mod sysinfo;
mod sysrc;
mod sysctl_conf;
mod terminal;
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
        tokio_accumulator: Arc::new(parking_lot::Mutex::new(Default::default())),
        login_guard: auth::LoginGuard::new(),
    };

    let user_count = {
        let conn = state.db.lock().await;
        crate::db::user_count(&conn)?
    };
    if user_count == 0 {
        tracing::warn!("no users yet — first-run setup required via the web UI");
    }

    // Safety check: if the previous process died with an unconfirmed firewall
    // change pending, roll it back now (the timer task was lost on restart).
    if let Some(p) = crate::firewall_gen::get_pending_apply() {
        if p.status == "pending" {
            tracing::warn!(
                "found unconfirmed firewall change on startup — rolling back"
            );
            let driver = p.driver;
            let backup = p.backup_config.clone();
            let was_enabled = p.was_enabled;
            if let Err(e) = tokio::task::spawn_blocking(move || {
                crate::firewall_gen::rollback(driver, &backup, was_enabled)
            })
            .await
            {
                tracing::error!(error = ?e, "startup rollback task failed");
            }
            crate::firewall_gen::clear_pending_apply();
        }
    }

    let app = build(state.clone());
    monitor::spawn_collector(state.clone());
    handlers::debug::spawn_tokio_accumulator(state);

    // Parse listen address.
    let addr: SocketAddr = config.server.listen.parse().map_err(|e| {
        anyhow::anyhow!("invalid listen address '{}': {}", config.server.listen, e)
    })?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "FWP listening (HTTP)");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}
