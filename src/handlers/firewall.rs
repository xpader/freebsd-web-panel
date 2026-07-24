//! Firewall management handlers — dual-driver (ipfw / pf) firewall with
//! structured rule CRUD, whitelist/blacklist mode, and rc.conf initialization.

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_confirm: Option<PendingConfirmInfo>,
}

#[derive(Debug, Serialize)]
pub struct PendingConfirmInfo {
    pub expires_at: i64,
    pub timeout_seconds: i64,
    pub operation: String,
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

/// Check if there are unapplied rule changes (staging file exists).
async fn is_dirty(_state: &AppState) -> bool {
    fw::has_staging()
}

/// Get effective rules + tables + nat_rules: from staging if exists, else from DB.
async fn effective_state(state: &AppState) -> ApiResult<(Vec<fw::FirewallRule>, Vec<fw::IpTable>, Vec<fw::NatRule>)> {
    if let Some((rules, tables, nat_rules)) = fw::read_staging() {
        return Ok((rules, tables, nat_rules));
    }
    let conn = state.db.lock().await;
    Ok((fw::list_rules(&conn)?, fw::list_tables(&conn)?, fw::list_nat_rules(&conn)?))
}

/// Check if firewall is currently enabled.
async fn is_fw_enabled(driver: FirewallDriver) -> bool {
    tokio::task::spawn_blocking(move || fw::is_firewall_enabled(driver))
        .await
        .unwrap_or(false)
}

/// Regenerate config file from DB (no kernel reload). Used when FW disabled.
async fn regen_config(state: &AppState, driver: FirewallDriver) -> ApiResult<()> {
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);
    let (rules, tables, nat_rules) = {
        let conn = state.db.lock().await;
        (fw::list_rules(&conn)?, fw::list_tables(&conn)?, fw::list_nat_rules(&conn)?)
    };
    let d = driver;
    tokio::task::spawn_blocking(move || fw::write_config_only(d, &rules, mode, &tables, &nat_rules))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;
    Ok(())
}

/// Check for a pending apply confirmation.
async fn get_pending_confirm() -> Option<PendingConfirmInfo> {
    let p = fw::get_pending_apply()?;
    if p.status != "pending" {
        return None;
    }
    Some(PendingConfirmInfo {
        expires_at: p.expires_at,
        timeout_seconds: fw::APPLY_TIMEOUT_SECS,
        operation: p.operation,
    })
}

/// Reject if there is an unconfirmed pending apply.
async fn check_no_pending() -> ApiResult<()> {
    if get_pending_confirm().await.is_some() {
        return Err(ApiError::Conflict(
            "firewall change pending confirmation".into(),
        ));
    }
    Ok(())
}

/// Spawn the auto-rollback timer for a pending apply.
/// Spawn the auto-rollback timer for a pending apply.
fn spawn_rollback_timer(db: crate::db::Db) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(
            fw::APPLY_TIMEOUT_SECS as u64,
        ))
        .await;

        if let Some(p) = fw::get_pending_apply() {
            if p.status == "pending" {
                tracing::warn!("firewall apply timeout — auto-rolling back");
                let driver = p.driver;
                let backup = p.backup_config.clone();
                let was_enabled = p.was_enabled;
                let old_mode = p.old_mode.clone();
                let result = tokio::task::spawn_blocking(move || {
                    fw::rollback(driver, &backup, was_enabled)
                })
                .await;

                // Restore old mode in DB if this was a mode-change operation.
                if let Some(om) = old_mode {
                    let conn = db.lock().await;
                    let _ = fw::set_state(&conn, "mode", &om);
                }

                fw::clear_pending_apply();
                fw::clear_staging();

                if let Err(e) = result {
                    tracing::error!(error = ?e, "rollback task panicked");
                }
            }
        }
    });
}

// ── handlers ───────────────────────────────────────────────────────

/// GET /api/firewall/status
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await;
    let mode = active_mode(&state).await;

    let (enabled, module_loaded, rules_count) = match driver {
        Some(d) => {
            let count = {
                let conn = state.db.lock().await;
                fw::count_enabled_rules(&conn)?
            };
            // Run blocking subprocess calls in spawn_blocking to avoid
            // stalling async worker threads.
            let (en, ml) = tokio::task::spawn_blocking(move || {
                (fw::is_firewall_enabled(d), d.module_loaded())
            })
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
            (en, ml, count)
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
        pending_confirm: get_pending_confirm().await,
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

    let (rules, tables, nat_rules): (Vec<fw::FirewallRule>, Vec<fw::IpTable>, Vec<fw::NatRule>) = {
        let conn = state.db.lock().await;
        (fw::list_rules(&conn)?, fw::list_tables(&conn)?, fw::list_nat_rules(&conn)?)
    };

    // Execute in blocking thread
    let driver_val = driver;
    let mode_val = mode;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        match driver_val {
            FirewallDriver::Ipfw => fw::init_ipfw(mode_val, &rules, &tables, &nat_rules)?,
            FirewallDriver::Pf => fw::init_pf(mode_val, &rules, &tables, &nat_rules)?,
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
            pending_confirm: None,
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

    // Load rules and tables
    let (new_rules, tables, nat_rules): (Vec<fw::FirewallRule>, Vec<fw::IpTable>, Vec<fw::NatRule>) = {
        let conn = state.db.lock().await;
        (fw::list_rules(&conn)?, fw::list_tables(&conn)?, fw::list_nat_rules(&conn)?)
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
        // Initialize the new driver, then load its rules while it remains disabled.
        match new {
            FirewallDriver::Ipfw => {
                fw::init_ipfw(mode_val, &new_rules, &tables, &nat_rules)?;
                fw::apply_ipfw(&new_rules, mode_val, &tables, &nat_rules)?;
            }
            FirewallDriver::Pf => {
                fw::init_pf(mode_val, &new_rules, &tables, &nat_rules)?;
                fw::apply_pf(&new_rules, mode_val, &tables, &nat_rules)?;
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
        pending_confirm: None,
    }))
}

/// POST /api/firewall/enable
pub async fn enable(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<axum::response::Response> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    check_no_pending().await?;

    // Clear any leftover staging and regenerate config from DB before enabling.
    fw::clear_staging();
    regen_config(&state, driver).await?;

    // Backup current config file before enabling.
    let backup_config = {
        let d = driver;
        tokio::task::spawn_blocking(move || fw::read_config_file(d))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    };

    let now = state.now_ts();
    fw::create_pending_apply("enable", driver, false, &backup_config, now)?;

    let d = driver;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        crate::sysrc::set_multi(&[
            ("firewall_enable", if d == FirewallDriver::Ipfw { "YES" } else { "NO" }),
            ("pf_enable", if d == FirewallDriver::Pf { "YES" } else { "NO" }),
        ]).map_err(|e| ApiError::Command(e))?;
        fw::enable_firewall(d)?;
        Ok(())
    })
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Start auto-rollback timer.
    spawn_rollback_timer(state.db.clone());

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/enable",
        200,
        Some(format!("enabled firewall ({driver:?}) — pending confirm")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok(([(header::CONNECTION, "close")], Json(FirewallStatus {
        driver: Some(driver),
        initialized: true,
        enabled: true,
        mode: Some(mode),
        module_loaded: true,
        rules_count: count,
        pending_apply: is_dirty(&state).await,
        pending_confirm: get_pending_confirm().await,
    })).into_response())
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
            FirewallDriver::Ipfw => crate::sysrc::ensure_no("firewall_enable"),
            FirewallDriver::Pf => crate::sysrc::ensure_no("pf_enable"),
        }
        Ok(())
    })
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Clear any pending staging when firewall is disabled.
    fw::clear_staging();

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
        pending_confirm: None,
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

    // Don't allow mode change if there's already a pending apply.
    check_no_pending().await?;

    let is_enabled = is_fw_enabled(driver).await;
    let old_mode = active_mode(&state).await;

    // Update DB mode.
    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "mode", new_mode.as_str())?;
    }

    if !is_enabled {
        // Firewall is disabled — just regenerate the config file, no apply.
        regen_config(&state, driver).await?;
        audit::record(
            &state, Some(&auth.username), "PUT", "/api/firewall/mode", 200,
            Some(format!("firewall mode -> {new_mode:?} (disabled, config only)")),
        );

        let conn = state.db.lock().await;
        let count = fw::count_enabled_rules(&conn)?;
        drop(conn);

        return Ok(Json(FirewallStatus {
            driver: Some(driver),
            initialized: true,
            enabled: false,
            mode: Some(new_mode),
            module_loaded: driver.module_loaded(),
            rules_count: count,
            pending_apply: is_dirty(&state).await,
            pending_confirm: None,
        }));
    }

    // Firewall is enabled — apply with anti-lockout.
    let (rules, tables, nat_rules) = effective_state(&state).await?;

    // Backup current config before applying.
    let backup_config = {
        let d = driver;
        tokio::task::spawn_blocking(move || fw::read_config_file(d))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    };

    let rules_clone = rules.clone();
    let mode_val = new_mode;
    let driver_val = driver;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        match driver_val {
            FirewallDriver::Ipfw => {
                fw::set_ipfw_mode(mode_val)?;
                fw::apply_ipfw(&rules_clone, mode_val, &tables, &nat_rules)?;
            }
            FirewallDriver::Pf => {
                fw::apply_pf(&rules_clone, mode_val, &tables, &nat_rules)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Set up pending confirmation + auto-rollback.
    let now = state.now_ts();
    fw::create_pending_apply("mode", driver, true, &backup_config, now)?;
    // Store old mode so rollback can restore it in DB.
    if let Some(ref om) = old_mode {
        fw::set_pending_old_mode(om.as_str())?;
    }
    spawn_rollback_timer(state.db.clone());

    let pending_confirm = Some(PendingConfirmInfo {
        expires_at: now + fw::APPLY_TIMEOUT_SECS,
        timeout_seconds: fw::APPLY_TIMEOUT_SECS,
        operation: "mode".to_string(),
    });

    audit::record(
        &state, Some(&auth.username), "PUT", "/api/firewall/mode", 200,
        Some(format!("firewall mode -> {new_mode:?} (enabled, anti-lockout active)")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    Ok(Json(FirewallStatus {
        driver: Some(driver),
        initialized: true,
        enabled: true,
        mode: Some(new_mode),
        module_loaded: true,
        rules_count: count,
        pending_apply: is_dirty(&state).await,
        pending_confirm,
    }))
}

/// GET /api/firewall/rules
pub async fn list_rules(State(state): State<AppState>) -> ApiResult<Json<Vec<fw::FirewallRule>>> {
    let _driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let (rules, _, _) = effective_state(&state).await?;
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

    if is_fw_enabled(driver).await {
        // FW enabled: modify staging
        let (mut rules, tables, nat_rules) = effective_state(&state).await?;
        let pos = rules.iter().map(|r| r.position).max().unwrap_or(0) + 1;
        let id = rules.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let rule = fw::FirewallRule {
            id, position: pos, enabled: true,
            action: body.action, direction: body.direction, protocol: body.protocol,
            source: body.source.clone(), source_port: body.source_port.clone(),
            destination: body.destination.clone(), destination_port: body.destination_port.clone(),
            interface: body.interface.clone(), log: body.log,
            icmp_type: body.icmp_type.clone(), description: body.description.clone(),
            created_at: now, updated_at: now,
        };
        rules.push(rule.clone());
        fw::write_staging(&rules, &tables, &nat_rules)?;
        audit::record(&state, Some(&auth.username), "POST", "/api/firewall/rules", 201,
            Some(format!("added firewall rule ({driver:?}) [staging]")));
        Ok((StatusCode::CREATED, Json(rule)))
    } else {
        // FW disabled: write DB + regen config
        let id = {
            let conn = state.db.lock().await;
            fw::create_rule(&conn, &body, now)?
        };
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "POST", "/api/firewall/rules", 201,
            Some(format!("added firewall rule ({driver:?})")));
        let conn = state.db.lock().await;
        let rule = fw::list_rules(&conn)?.into_iter().find(|r| r.id == id)
            .ok_or_else(|| ApiError::Internal("created rule not found".into()))?;
        Ok((StatusCode::CREATED, Json(rule)))
    }
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

    if is_fw_enabled(driver).await {
        let had_staging = fw::has_staging();
        let (mut rules, tables, nat_rules) = effective_state(&state).await?;
        let rule = rules.iter_mut().find(|r| r.id == id)
            .ok_or_else(|| ApiError::NotFound("firewall rule not found".into()))?;
        let disabled = !rule.enabled;
        rule.action = body.action;
        rule.direction = body.direction;
        rule.protocol = body.protocol;
        rule.source = body.source.clone();
        rule.source_port = body.source_port.clone();
        rule.destination = body.destination.clone();
        rule.destination_port = body.destination_port.clone();
        rule.interface = body.interface.clone();
        rule.log = body.log;
        rule.icmp_type = body.icmp_type.clone();
        rule.description = body.description.clone();
        rule.updated_at = now;
        let updated = rule.clone();
        if disabled {
            let conn = state.db.lock().await;
            fw::update_rule(&conn, id, &body, now)?;
            drop(conn);
            if had_staging {
                fw::write_staging(&rules, &tables, &nat_rules)?;
            }
            audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/rules/{id}"), 200,
                Some(format!("updated disabled firewall rule {id} ({driver:?})")));
        } else {
            fw::write_staging(&rules, &tables, &nat_rules)?;
            audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/rules/{id}"), 200,
                Some(format!("updated firewall rule {id} ({driver:?}) [staging]")));
        }
        Ok(Json(updated))
    } else {
        {
            let conn = state.db.lock().await;
            fw::update_rule(&conn, id, &body, now)?;
        }
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/rules/{id}"), 200,
            Some(format!("updated firewall rule {id} ({driver:?})")));
        let conn = state.db.lock().await;
        let rule = fw::list_rules(&conn)?.into_iter().find(|r| r.id == id)
            .ok_or_else(|| ApiError::Internal("updated rule not found".into()))?;
        Ok(Json(rule))
    }
}

/// DELETE /api/firewall/rules/{id}
pub async fn delete_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    if is_fw_enabled(driver).await {
        let had_staging = fw::has_staging();
        let (mut rules, tables, nat_rules) = effective_state(&state).await?;
        let index = rules.iter().position(|r| r.id == id)
            .ok_or_else(|| ApiError::NotFound("firewall rule not found".into()))?;
        let disabled = !rules[index].enabled;
        rules.remove(index);
        if disabled {
            let conn = state.db.lock().await;
            fw::delete_rule(&conn, id)?;
            drop(conn);
            if had_staging {
                fw::write_staging(&rules, &tables, &nat_rules)?;
            }
        } else {
            fw::write_staging(&rules, &tables, &nat_rules)?;
        }
    } else {
        {
            let conn = state.db.lock().await;
            fw::delete_rule(&conn, id)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "DELETE", &format!("/api/firewall/rules/{id}"), 200,
        Some(format!("deleted firewall rule {id} ({driver:?})")));
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

    if is_fw_enabled(driver).await {
        let (mut rules, tables, nat_rules) = effective_state(&state).await?;
        let rule = rules.iter_mut().find(|r| r.id == id)
            .ok_or_else(|| ApiError::NotFound("firewall rule not found".into()))?;
        rule.enabled = !rule.enabled;
        fw::write_staging(&rules, &tables, &nat_rules)?;
    } else {
        {
            let conn = state.db.lock().await;
            fw::toggle_rule(&conn, id)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/rules/{id}/toggle"), 200,
        Some(format!("toggled firewall rule {id} ({driver:?})")));
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

    if is_fw_enabled(driver).await {
        let (mut rules, tables, nat_rules) = effective_state(&state).await?;
        let mut reordered = Vec::with_capacity(body.ordered_ids.len());
        for id in &body.ordered_ids {
            if let Some(pos) = rules.iter().position(|r| &r.id == id) {
                reordered.push(rules.remove(pos));
            }
        }
        for (i, rule) in reordered.iter_mut().enumerate() {
            rule.position = i as u32;
        }
        fw::write_staging(&reordered, &tables, &nat_rules)?;
    } else {
        {
            let conn = state.db.lock().await;
            fw::reorder_rules(&conn, &body.ordered_ids)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "PUT", "/api/firewall/rules/reorder", 200,
        Some(format!("reordered firewall rules ({driver:?})")));
    Ok(StatusCode::OK)
}

/// POST /api/firewall/apply
pub async fn apply(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<axum::response::Response> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    check_no_pending().await?;

    // Read effective rules (staging if exists, else DB)
    let (rules, tables, nat_rules) = effective_state(&state).await?;

    // Check if firewall is currently enabled — if so, we need anti-lockout.
    let d0 = driver;
    let is_enabled = tokio::task::spawn_blocking(move || fw::is_firewall_enabled(d0))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;

    // Backup current config before applying.
    let backup_config = {
        let d = driver;
        tokio::task::spawn_blocking(move || fw::read_config_file(d))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    };

    let rules_clone = rules.clone();
    let mode_val = mode;
    let driver_val = driver;
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        match driver_val {
            FirewallDriver::Ipfw => fw::apply_ipfw(&rules_clone, mode_val, &tables, &nat_rules)?,
            FirewallDriver::Pf => fw::apply_pf(&rules_clone, mode_val, &tables, &nat_rules)?,
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "rules_dirty", "0")?;
    }

    // If firewall was enabled, set up pending confirmation + auto-rollback.
    let pending_confirm = if is_enabled {
        let now = state.now_ts();
        fw::create_pending_apply("apply", driver, true, &backup_config, now)?;
        spawn_rollback_timer(state.db.clone());

        serde_json::json!({
            "expires_at": now + fw::APPLY_TIMEOUT_SECS,
            "timeout_seconds": fw::APPLY_TIMEOUT_SECS,
            "operation": "apply",
        })
    } else {
        serde_json::Value::Null
    };

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/apply",
        200,
        Some(format!("applied firewall rules ({driver:?})")),
    );

    Ok(([(header::CONNECTION, "close")], Json(serde_json::json!({
        "applied": true,
        "driver": driver,
        "rules_count": rules.iter().filter(|r| r.enabled).count(),
        "pending_confirm": pending_confirm,
    }))).into_response())
}

/// GET /api/firewall/config
pub async fn config(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let (rules, tables, nat_rules) = effective_state(&state).await?;

    // Generate preview from effective state (staging or DB)
    let content = fw::preview_config(driver, &rules, mode, &tables, &nat_rules);

    Ok(Json(serde_json::json!({
        "driver": driver,
        "mode": mode,
        "content": content,
    })))
}

// ── IP table handlers ──────────────────────────────────────────────

/// GET /api/firewall/tables
pub async fn list_tables(State(state): State<AppState>) -> ApiResult<Json<Vec<fw::IpTable>>> {
    let (_, tables, _) = effective_state(&state).await?;
    Ok(Json(tables))
}

/// POST /api/firewall/tables
pub async fn create_table(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<fw::TableBody>,
) -> ApiResult<(StatusCode, Json<fw::IpTable>)> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    fw::validate_table_name(&body.name)?;
    let now = state.now_ts();

    if is_fw_enabled(driver).await {
        let (rules, mut tables, nat_rules) = effective_state(&state).await?;
        let id = tables.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let table = fw::IpTable {
            id, name: body.name.clone(), description: body.description.clone(),
            entries: Vec::new(), created_at: now, updated_at: now,
        };
        tables.push(table.clone());
        fw::write_staging(&rules, &tables, &nat_rules)?;
        audit::record(&state, Some(&auth.username), "POST", "/api/firewall/tables", 201,
            Some(format!("created firewall table '{}' [staging]", body.name)));
        Ok((StatusCode::CREATED, Json(table)))
    } else {
        let id = {
            let conn = state.db.lock().await;
            fw::create_table(&conn, &body, now)?
        };
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "POST", "/api/firewall/tables", 201,
            Some(format!("created firewall table '{}'", body.name)));
        let conn = state.db.lock().await;
        let table = fw::list_tables(&conn)?.into_iter().find(|t| t.id == id)
            .ok_or_else(|| ApiError::Internal("created table not found".into()))?;
        Ok((StatusCode::CREATED, Json(table)))
    }
}

/// PUT /api/firewall/tables/{id}
pub async fn update_table(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<fw::TableBody>,
) -> ApiResult<Json<fw::IpTable>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    fw::validate_table_name(&body.name)?;
    let now = state.now_ts();

    if is_fw_enabled(driver).await {
        let (rules, mut tables, nat_rules) = effective_state(&state).await?;
        let table = tables.iter_mut().find(|t| t.id == id)
            .ok_or_else(|| ApiError::NotFound("firewall table not found".into()))?;
        table.name = body.name.clone();
        table.description = body.description.clone();
        table.updated_at = now;
        let updated = table.clone();
        fw::write_staging(&rules, &tables, &nat_rules)?;
        audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/tables/{id}"), 200,
            Some(format!("updated firewall table {id} [staging]")));
        Ok(Json(updated))
    } else {
        {
            let conn = state.db.lock().await;
            fw::update_table(&conn, id, &body, now)?;
        }
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/tables/{id}"), 200,
            Some(format!("updated firewall table {id}")));
        let conn = state.db.lock().await;
        let table = fw::list_tables(&conn)?.into_iter().find(|t| t.id == id)
            .ok_or_else(|| ApiError::Internal("updated table not found".into()))?;
        Ok(Json(table))
    }
}

/// DELETE /api/firewall/tables/{id}
pub async fn delete_table(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    if is_fw_enabled(driver).await {
        let (rules, mut tables, nat_rules) = effective_state(&state).await?;
        tables.retain(|t| t.id != id);
        fw::write_staging(&rules, &tables, &nat_rules)?;
    } else {
        {
            let conn = state.db.lock().await;
            fw::delete_table(&conn, id)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "DELETE", &format!("/api/firewall/tables/{id}"), 200,
        Some(format!("deleted firewall table {id}")));
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/firewall/tables/{id}/entries
pub async fn add_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<fw::EntryBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    fw::validate_address(&body.address)?;
    let now = state.now_ts();

    if is_fw_enabled(driver).await {
        let (rules, mut tables, nat_rules) = effective_state(&state).await?;
        let table = tables.iter_mut().find(|t| t.id == id)
            .ok_or_else(|| ApiError::NotFound("firewall table not found".into()))?;
        let entry_id = table.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        table.entries.push(fw::IpTableEntry {
            id: entry_id, table_id: id, address: body.address.clone(), created_at: now,
        });
        fw::write_staging(&rules, &tables, &nat_rules)?;
        audit::record(&state, Some(&auth.username), "POST", &format!("/api/firewall/tables/{id}/entries"), 201,
            Some(format!("added entry '{}' to table {id} [staging]", body.address)));
        Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": entry_id, "address": body.address }))))
    } else {
        let entry_id = {
            let conn = state.db.lock().await;
            let tables = fw::list_tables(&conn)?;
            if !tables.iter().any(|t| t.id == id) {
                return Err(ApiError::NotFound("firewall table not found".into()));
            }
            fw::add_entry(&conn, id, &body.address, now)?
        };
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "POST", &format!("/api/firewall/tables/{id}/entries"), 201,
            Some(format!("added entry '{}' to table {id}", body.address)));
        Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": entry_id, "address": body.address }))))
    }
}

/// DELETE /api/firewall/tables/{id}/entries/{eid}
pub async fn delete_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, eid)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    if is_fw_enabled(driver).await {
        let (rules, mut tables, nat_rules) = effective_state(&state).await?;
        let table = tables.iter_mut().find(|t| t.id == id)
            .ok_or_else(|| ApiError::NotFound("firewall table not found".into()))?;
        table.entries.retain(|e| e.id != eid);
        fw::write_staging(&rules, &tables, &nat_rules)?;
    } else {
        {
            let conn = state.db.lock().await;
            fw::delete_entry(&conn, id, eid)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "DELETE", &format!("/api/firewall/tables/{id}/entries/{eid}"), 200,
        Some(format!("deleted entry {eid} from table {id}")));
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/firewall/confirm
pub async fn confirm(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let pending = fw::get_pending_apply();
    if pending.is_none() || pending.as_ref().unwrap().status != "pending" {
        return Err(ApiError::BadRequest("no pending firewall change".into()));
    }
    fw::clear_pending_apply();

    // Commit staging to DB if staging exists.
    if let Some((rules, tables, nat_rules)) = fw::read_staging() {
        let conn = state.db.lock().await;
        fw::replace_all_rules(&conn, &rules)?;
        fw::replace_all_tables(&conn, &tables)?;
        fw::replace_all_nat_rules(&conn, &nat_rules)?;
        drop(conn);
        fw::clear_staging();
    }

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/confirm",
        200,
        Some(format!("confirmed firewall change ({driver:?})")),
    );

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    let (enabled, module_loaded) = {
        let d = driver;
        tokio::task::spawn_blocking(move || (fw::is_firewall_enabled(d), d.module_loaded()))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    };

    Ok(Json(FirewallStatus {
        driver: Some(driver),
        initialized: true,
        enabled,
        mode: Some(mode),
        module_loaded,
        rules_count: count,
        pending_apply: is_dirty(&state).await,
        pending_confirm: None,
    }))
}

/// POST /api/firewall/rollback
pub async fn rollback(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<FirewallStatus>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    // If no pending file exists, the auto-rollback timer already fired.
    // Return current status instead of erroring (idempotent rollback).
    let p = match fw::get_pending_apply().filter(|p| p.status == "pending") {
        Some(p) => p,
        None => {
            let conn = state.db.lock().await;
            let count = fw::count_enabled_rules(&conn)?;
            drop(conn);
            let (enabled, module_loaded) = {
                let d = driver;
                tokio::task::spawn_blocking(move || (fw::is_firewall_enabled(d), d.module_loaded()))
                    .await
                    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
            };
            return Ok(Json(FirewallStatus {
                driver: Some(driver),
                initialized: true,
                enabled,
                mode: Some(mode),
                module_loaded,
                rules_count: count,
                pending_apply: is_dirty(&state).await,
                pending_confirm: None,
            }));
        }
    };

    let rollback_driver = p.driver;
    let backup = p.backup_config.clone();
    let was_enabled = p.was_enabled;
    let old_mode = p.old_mode.clone();
    tokio::task::spawn_blocking(move || fw::rollback(rollback_driver, &backup, was_enabled))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Restore old mode in DB if this was a mode-change operation.
    if let Some(om) = old_mode {
        let conn = state.db.lock().await;
        fw::set_state(&conn, "mode", &om)?;
    }

    fw::clear_pending_apply();
    fw::clear_staging();

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/rollback",
        200,
        Some(format!("rolled back firewall change ({driver:?})")),
    );

    // Re-read mode from DB — it may have been restored if this was a mode rollback.
    let restored_mode = active_mode(&state).await.unwrap_or(FirewallMode::Whitelist);

    let conn = state.db.lock().await;
    let count = fw::count_enabled_rules(&conn)?;
    drop(conn);

    let (enabled, module_loaded) = {
        let d = p.driver;
        tokio::task::spawn_blocking(move || (fw::is_firewall_enabled(d), d.module_loaded()))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    };

    Ok(Json(FirewallStatus {
        driver: Some(p.driver),
        initialized: true,
        enabled,
        mode: Some(restored_mode),
        module_loaded,
        rules_count: count,
        pending_apply: is_dirty(&state).await,
        pending_confirm: None,
    }))
}

/// POST /api/firewall/discard — discard uncommitted staging changes.
pub async fn discard(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<StatusCode> {
    fw::clear_staging();
    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/firewall/discard",
        200,
        Some("discarded uncommitted firewall changes".into()),
    );
    Ok(StatusCode::OK)
}

// ── NAT rule handlers ──────────────────────────────────────────────

/// GET /api/firewall/nat/rules
pub async fn list_nat_rules(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<fw::NatRule>>> {
    let _driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    let (_, _, nat_rules) = effective_state(&state).await?;
    Ok(Json(nat_rules))
}

/// POST /api/firewall/nat/rules
pub async fn create_nat_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<fw::NatBody>,
) -> ApiResult<(StatusCode, Json<fw::NatRule>)> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    fw::validate_nat_body(&body)?;
    let now = state.now_ts();

    if is_fw_enabled(driver).await {
        // FW enabled: modify staging (nat_rules portion)
        let (rules, tables, mut nat_rules) = effective_state(&state).await?;
        let id = nat_rules.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let pos = nat_rules.iter().map(|r| r.position).max().unwrap_or(0) + 1;
        let rule = fw::NatRule {
            id,
            position: pos,
            enabled: body.enabled,
            kind: body.kind,
            family: body.family,
            interface: body.interface.clone(),
            src_addr: body.src_addr.clone(),
            dst_addr: body.dst_addr.clone(),
            src_port: body.src_port.clone(),
            dst_port: body.dst_port.clone(),
            protocol: body.protocol,
            description: body.description.clone(),
            created_at: now,
            updated_at: now,
        };
        nat_rules.push(rule.clone());
        fw::write_staging(&rules, &tables, &nat_rules)?;
        audit::record(&state, Some(&auth.username), "POST", "/api/firewall/nat/rules", 201,
            Some(format!("created NAT rule [staging]: kind={:?} iface={}", body.kind, body.interface)));
        Ok((StatusCode::CREATED, Json(rule)))
    } else {
        let id = {
            let conn = state.db.lock().await;
            fw::create_nat_rule(&conn, &body, now)?
        };
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "POST", "/api/firewall/nat/rules", 201,
            Some(format!("created NAT rule: kind={:?} iface={}", body.kind, body.interface)));
        let conn = state.db.lock().await;
        let rule = fw::list_nat_rules(&conn)?.into_iter().find(|r| r.id == id)
            .ok_or_else(|| ApiError::Internal("created NAT rule not found".into()))?;
        Ok((StatusCode::CREATED, Json(rule)))
    }
}

/// PUT /api/firewall/nat/rules/{id}
pub async fn update_nat_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<fw::NatBody>,
) -> ApiResult<Json<fw::NatRule>> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;
    fw::validate_nat_body(&body)?;
    let now = state.now_ts();

    if is_fw_enabled(driver).await {
        let had_staging = fw::has_staging();
        let (rules, tables, mut nat_rules) = effective_state(&state).await?;
        let rule = nat_rules.iter_mut().find(|r| r.id == id)
            .ok_or_else(|| ApiError::NotFound("NAT rule not found".into()))?;
        let disabled = !rule.enabled;
        rule.kind = body.kind;
        rule.family = body.family;
        rule.interface = body.interface.clone();
        rule.src_addr = body.src_addr.clone();
        rule.dst_addr = body.dst_addr.clone();
        rule.src_port = body.src_port.clone();
        rule.dst_port = body.dst_port.clone();
        rule.protocol = body.protocol;
        rule.enabled = body.enabled;
        rule.description = body.description.clone();
        rule.updated_at = now;
        let updated = rule.clone();
        if disabled {
            let conn = state.db.lock().await;
            fw::update_nat_rule(&conn, id, &body, now)?;
            drop(conn);
            if had_staging {
                fw::write_staging(&rules, &tables, &nat_rules)?;
            }
            audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/nat/rules/{id}"), 200,
                Some(format!("updated disabled NAT rule {id}")));
        } else {
            fw::write_staging(&rules, &tables, &nat_rules)?;
            audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/nat/rules/{id}"), 200,
                Some(format!("updated NAT rule {id} [staging]")));
        }
        Ok(Json(updated))
    } else {
        {
            let conn = state.db.lock().await;
            fw::update_nat_rule(&conn, id, &body, now)?;
        }
        regen_config(&state, driver).await?;
        audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/nat/rules/{id}"), 200,
            Some(format!("updated NAT rule {id}")));
        let conn = state.db.lock().await;
        let rule = fw::list_nat_rules(&conn)?.into_iter().find(|r| r.id == id)
            .ok_or_else(|| ApiError::Internal("updated NAT rule not found".into()))?;
        Ok(Json(rule))
    }
}

/// DELETE /api/firewall/nat/rules/{id}
pub async fn delete_nat_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    if is_fw_enabled(driver).await {
        let had_staging = fw::has_staging();
        let (rules, tables, mut nat_rules) = effective_state(&state).await?;
        let index = nat_rules.iter().position(|r| r.id == id)
            .ok_or_else(|| ApiError::NotFound("NAT rule not found".into()))?;
        let disabled = !nat_rules[index].enabled;
        nat_rules.remove(index);
        if disabled {
            let conn = state.db.lock().await;
            fw::delete_nat_rule(&conn, id)?;
            drop(conn);
            if had_staging {
                fw::write_staging(&rules, &tables, &nat_rules)?;
            }
        } else {
            fw::write_staging(&rules, &tables, &nat_rules)?;
        }
    } else {
        {
            let conn = state.db.lock().await;
            fw::delete_nat_rule(&conn, id)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "DELETE", &format!("/api/firewall/nat/rules/{id}"), 200,
        Some(format!("deleted NAT rule {id}")));
    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/firewall/nat/rules/{id}/toggle
pub async fn toggle_nat_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    if is_fw_enabled(driver).await {
        let (rules, tables, mut nat_rules) = effective_state(&state).await?;
        let rule = nat_rules.iter_mut().find(|r| r.id == id)
            .ok_or_else(|| ApiError::NotFound("NAT rule not found".into()))?;
        rule.enabled = !rule.enabled;
        fw::write_staging(&rules, &tables, &nat_rules)?;
    } else {
        {
            let conn = state.db.lock().await;
            fw::toggle_nat_rule(&conn, id)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "PUT", &format!("/api/firewall/nat/rules/{id}/toggle"), 200,
        Some(format!("toggled NAT rule {id}")));
    Ok(StatusCode::OK)
}

/// PUT /api/firewall/nat/rules/reorder
pub async fn reorder_nat_rules(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReorderBody>,
) -> ApiResult<StatusCode> {
    let driver = active_driver(&state).await
        .ok_or_else(|| ApiError::BadRequest("firewall not initialized".into()))?;

    if is_fw_enabled(driver).await {
        let (rules, tables, mut nat_rules) = effective_state(&state).await?;
        let mut reordered = Vec::with_capacity(body.ordered_ids.len());
        for id in &body.ordered_ids {
            if let Some(pos) = nat_rules.iter().position(|r| &r.id == id) {
                reordered.push(nat_rules.remove(pos));
            }
        }
        for (i, rule) in reordered.iter_mut().enumerate() {
            rule.position = i as u32;
        }
        fw::write_staging(&rules, &tables, &reordered)?;
    } else {
        {
            let conn = state.db.lock().await;
            fw::reorder_nat_rules(&conn, &body.ordered_ids)?;
        }
        regen_config(&state, driver).await?;
    }

    audit::record(&state, Some(&auth.username), "PUT", "/api/firewall/nat/rules/reorder", 200,
        Some(format!("reordered NAT rules ({driver:?})")));
    Ok(StatusCode::OK)
}
