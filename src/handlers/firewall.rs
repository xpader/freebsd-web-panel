//! Firewall management handlers — dual-driver (ipfw / pf) firewall with
//! structured rule CRUD, whitelist/blacklist mode, and rc.conf initialization.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::error::{ApiError, ApiResult};
use crate::firewall_gen as fw;
use crate::firewall_gen::{FirewallDriver, FirewallMode};
use crate::auth::AuthUser;
use crate::state::AppState;

// ── request/response types ─────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct FirewallStatus {
    pub driver: Option<FirewallDriver>,
    pub initialized: bool,
    pub enabled: bool,
    pub mode: Option<FirewallMode>,
    pub module_loaded: bool,
    pub rules_count: i64,
    pub pending_apply: bool,
}

#[derive(Debug, Deserialize)]
pub struct InitializeBody {
    pub driver: FirewallDriver,
    pub mode: FirewallMode,
}

#[derive(Debug, Deserialize)]
pub struct SwitchBody {
    pub driver: FirewallDriver,
}

#[derive(Debug, Deserialize)]
pub struct ModeBody {
    pub mode: FirewallMode,
}

#[derive(Debug, Deserialize)]
pub struct ReorderBody {
    pub ordered_ids: Vec<i64>,
}

// ── helpers ────────────────────────────────────────────────────────

/// Read active driver from DB.
async fn active_driver(state: &AppState) -> Option<FirewallDriver> {
    let conn = state.db.lock().await;
    fw::get_state(&conn, "active_driver")
        .as_deref()
        .and_then(FirewallDriver::from_str)
}

/// Read active mode from DB.
async fn active_mode(state: &AppState) -> Option<FirewallMode> {
    let conn = state.db.lock().await;
    fw::get_state(&conn, "mode")
        .as_deref()
        .and_then(FirewallMode::from_str)
}

/// Check if there are unapplied rule changes.
async fn is_dirty(state: &AppState) -> bool {
    let conn = state.db.lock().await;
    fw::get_state(&conn, "rules_dirty")
        .map(|v| v == "1")
        .unwrap_or(false)
}

// ── handlers ───────────────────────────────────────────────────────

/// GET /api/firewall/status
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await;
    let mode = active_mode(&state).await;

    let (enabled, module_loaded, rules_count) = match driver {
        Some(d) => {
            let conn = state.db.lock().await;
            let count = fw::count_enabled_rules(&conn)?;
            drop(conn);
            (
                fw::is_firewall_enabled(d),
                d.module_loaded(),
                count,
            )
        }
        None => (false, false, 0),
    };

    Ok(Json(FirewallStatus {
        driver,
        initialized: driver.is_some(),
        enabled,
        mode,
        module_loaded,
        rules_count,
        pending_apply: is_dirty(&state).await,
    }))
}

/// POST /api/firewall/initialize
pub async fn initialize(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<InitializeBody>,
) -> ApiResult<(StatusCode, Json<FirewallStatus>)> {
    if active_driver(&state).await.is_some() {
        return Err(ApiError::Conflict("firewall already initialized".into()));
    }

    let driver = body.driver;
    let mode = body.mode;

    let rules: Vec<fw::FirewallRule> = {
        let conn = state.db.lock().await;
        fw::list_rules(&conn)?
    };

    // Execute in blocking thread
    let driver_val = driver;
    let mode_val = mode;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        match driver_val {
            FirewallDriver::Ipfw => fw::init_ipfw(mode_val, &rules)?,
            FirewallDriver::Pf => fw::init_pf(mode_val, &rules)?,
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Save state
    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "active_driver", driver.as_str())?;
        fw::set_state(&conn, "mode", mode.as_str())?;
        fw::set_state(&conn, "rules_dirty", "0")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/initialize",
        200,
        Some(format!("initialized firewall: driver={driver:?} mode={mode:?}")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok((
        StatusCode::CREATED,
        Json(FirewallStatus {
            driver: Some(driver),
            initialized: true,
            enabled: false,
            mode: Some(mode),
            module_loaded: true,
            rules_count: count,
            pending_apply: false,
        }),
    ))
}

/// POST /api/firewall/switch
pub async fn switch(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SwitchBody>,
) -> ApiResult<Json<FirewallStatus>> {
    let old_driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let new_driver = body.driver;

    if old_driver == new_driver {
        return Err(ApiError::BadRequest("already using this driver".into()));
    }

    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    // Load new driver's rules
    let new_rules: Vec<fw::FirewallRule> = {
        let conn = state.db.lock().await;
        fw::list_rules(&conn)?
    };

    let old = old_driver;
    let new = new_driver;
    let mode_val = mode;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        // Deactivate old driver
        match old {
            FirewallDriver::Ipfw => fw::deactivate_ipfw()?,
            FirewallDriver::Pf => fw::deactivate_pf()?,
        }
        // Initialize new driver (rc.conf + module + config file)
        match new {
            FirewallDriver::Ipfw => {
                fw::init_ipfw(mode_val, &new_rules)?;
                fw::apply_ipfw(&new_rules, mode_val)?;
            }
            FirewallDriver::Pf => {
                fw::init_pf(mode_val, &new_rules)?;
                fw::apply_pf(&new_rules, mode_val)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Update DB state
    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "active_driver", new_driver.as_str())?;
        fw::set_state(&conn, "rules_dirty", "0")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/switch",
        200,
        Some(format!("switched firewall driver: {old:?} -> {new_driver:?}")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok(Json(FirewallStatus {
        driver: Some(new_driver),
        initialized: true,
        enabled: false,
        mode: Some(mode),
        module_loaded: true,
        rules_count: count,
        pending_apply: false,
    }))
}

/// POST /api/firewall/enable
pub async fn enable(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let d = driver;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        // Ensure active driver rc.conf is YES, inactive is NO
        crate::sysrc::set("firewall_enable", if d == FirewallDriver::Ipfw { "YES" } else { "NO" })
            .map_err(|e| ApiError::Command(e))?;
        crate::sysrc::set("pf_enable", if d == FirewallDriver::Pf { "YES" } else { "NO" })
            .map_err(|e| ApiError::Command(e))?;
        fw::enable_firewall(d)?;
        Ok(())
    })
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/enable",
        200,
        Some(format!("enabled firewall ({driver:?})")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok(Json(FirewallStatus {
        driver: Some(driver),
        initialized: true,
        enabled: true,
        mode: Some(mode),
        module_loaded: true,
        rules_count: count,
        pending_apply: is_dirty(&state).await,
    }))
}

/// POST /api/firewall/disable
pub async fn disable(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let d = driver;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        fw::disable_firewall(d)?;
        // Also set rc.conf to NO so it won't start on boot
        match d {
            FirewallDriver::Ipfw => {
                crate::sysrc::set("firewall_enable", "NO").map_err(|e| ApiError::Command(e))?;
            }
            FirewallDriver::Pf => {
                crate::sysrc::set("pf_enable", "NO").map_err(|e| ApiError::Command(e))?;
            }
        }
        Ok(())
    })
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/disable",
        200,
        Some(format!("disabled firewall ({driver:?})")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok(Json(FirewallStatus {
        driver: Some(driver),
        initialized: true,
        enabled: false,
        mode: Some(mode),
        module_loaded: true,
        rules_count: count,
        pending_apply: is_dirty(&state).await,
    }))
}

/// PUT /api/firewall/mode
pub async fn set_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ModeBody>,
) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let new_mode = body.mode;

    // Update DB state first
    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "mode", new_mode.as_str())?;
    }

    // Load rules and regenerate config with new mode
    let rules: Vec<fw::FirewallRule> = {
        let conn = state.db.lock().await;
        fw::list_rules(&conn)?
    };

    let driver_val = driver;
    let mode_val = new_mode;
    let rules_clone = rules.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        match driver_val {
            FirewallDriver::Ipfw => {
                // Persist boot-time default via loader.conf
                fw::set_ipfw_mode(mode_val)?;
                // Reload rules (includes default deny rule 65534 for whitelist)
                fw::apply_ipfw(&rules_clone, mode_val)?;
            }
            FirewallDriver::Pf => {
                // Regenerate pf.conf with new mode and reload
                fw::apply_pf(&rules_clone, mode_val)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/firewall/mode",
        200,
        Some(format!("firewall mode -> {new_mode:?}")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok(Json(FirewallStatus {
        driver: Some(driver),
        initialized: true,
        enabled: fw::is_firewall_enabled(driver),
        mode: Some(new_mode),
        module_loaded: true,
        rules_count: count,
        pending_apply: false,
    }))
}

/// GET /api/firewall/rules
pub async fn list_rules(State(state): State<AppState>) -> ApiResult<Json<Vec<fw::FirewallRule>>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let conn = state.db.lock().await;
    let rules = fw::list_rules(&conn)?;
    Ok(Json(rules))
}

/// POST /api/firewall/rules
pub async fn create_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<fw::RuleBody>,
) -> ApiResult<(StatusCode, Json<fw::FirewallRule>)> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    fw::validate_rule_body(&body)?;

    let now = state.now_ts();
    let id = {
        let conn = state.db.lock().await;
        fw::create_rule(&conn, &body, now)?
    };

    // Mark dirty
    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "rules_dirty", "1")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/rules",
        201,
        Some(format!("added firewall rule ({driver:?})")),
    );

    let conn = state.db.lock().await;
    let rule = fw::list_rules(&conn)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::Internal("created rule not found".into()))?;
    drop(conn);

    Ok((StatusCode::CREATED, Json(rule)))
}

/// PUT /api/firewall/rules/{id}
pub async fn update_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<fw::RuleBody>,
) -> ApiResult<Json<fw::FirewallRule>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    fw::validate_rule_body(&body)?;

    let now = state.now_ts();
    {
        let conn = state.db.lock().await;
        fw::update_rule(&conn, id, &body, now)?;
        fw::set_state(&conn, "rules_dirty", "1")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        &format!("/api/firewall/rules/{id}"),
        200,
        Some(format!("updated firewall rule {id} ({driver:?})")),
    );

    let conn = state.db.lock().await;
    let rule = fw::list_rules(&conn)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::Internal("updated rule not found".into()))?;
    drop(conn);

    Ok(Json(rule))
}

/// DELETE /api/firewall/rules/{id}
pub async fn delete_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    {
        let conn = state.db.lock().await;
        fw::delete_rule(&conn, id)?;
        fw::set_state(&conn, "rules_dirty", "1")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "DELETE",
        &format!("/api/firewall/rules/{id}"),
        200,
        Some(format!("deleted firewall rule {id} ({driver:?})")),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/firewall/rules/{id}/toggle
pub async fn toggle_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    {
        let conn = state.db.lock().await;
        fw::toggle_rule(&conn, id)?;
        fw::set_state(&conn, "rules_dirty", "1")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        &format!("/api/firewall/rules/{id}/toggle"),
        200,
        Some(format!("toggled firewall rule {id} ({driver:?})")),
    );

    Ok(StatusCode::OK)
}

/// PUT /api/firewall/rules/reorder
pub async fn reorder_rules(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReorderBody>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    {
        let conn = state.db.lock().await;
        fw::reorder_rules(&conn, &body.ordered_ids)?;
        fw::set_state(&conn, "rules_dirty", "1")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/firewall/rules/reorder",
        200,
        Some(format!("reordered firewall rules ({driver:?})")),
    );

    Ok(StatusCode::OK)
}

/// POST /api/firewall/apply
pub async fn apply(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let rules: Vec<fw::FirewallRule> = {
        let conn = state.db.lock().await;
        fw::list_rules(&conn)?
    };

    let rules_clone = rules.clone();
    let mode_val = mode;
    let driver_val = driver;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        match driver_val {
            FirewallDriver::Ipfw => fw::apply_ipfw(&rules_clone, mode_val)?,
            FirewallDriver::Pf => fw::apply_pf(&rules_clone, mode_val)?,
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "rules_dirty", "0")?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/apply",
        200,
        Some(format!("applied firewall rules ({driver:?})")),
    );

    Ok(Json(serde_json::json!({
        "applied": true,
        "driver": driver,
        "rules_count": rules.iter().filter(|r| r.enabled).count(),
    })))
}

/// GET /api/firewall/config
pub async fn config(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let rules: Vec<fw::FirewallRule> = {
        let conn = state.db.lock().await;
        fw::list_rules(&conn)?
    };

    // Generate preview from DB (not from file, so it reflects pending changes)
    let content = fw::preview_config(driver, &rules, mode);

    Ok(Json(serde_json::json!({
        "driver": driver,
        "mode": mode,
        "content": content,
    })))
}
