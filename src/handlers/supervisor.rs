//! HTTP handlers for the process supervisor. Core logic (spawn/stop/status/
//! validation/logs) lives in `src/supervisor.rs`; this layer is thin:
//! deserialize → validate (in core) → run core op on a blocking thread →
//! audit → respond.

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::supervisor::{self, ProcDef, ProcReq, ProcStatus};
use crate::AppState;

/// Definition + runtime state, as returned by the list endpoint.
#[derive(Debug, serde::Serialize)]
pub struct ProcInfo {
    #[serde(flatten)]
    pub def: ProcDef,
    pub state: &'static str,
    pub sup_pid: Option<i32>,
    pub pid: Option<i32>,
    pub started_at: Option<i64>,
    pub uptime: Option<i64>,
}

impl ProcInfo {
    fn build(dir: &std::path::Path, def: ProcDef, now: i64) -> Self {
        let ProcStatus {
            state,
            sup_pid,
            pid,
            started_at,
        } = supervisor::status(dir, &def.name);
        let uptime = started_at.map(|s| (now - s).max(0));
        Self {
            def,
            state,
            sup_pid,
            pid,
            started_at,
            uptime,
        }
    }
}

async fn get_def(state: &AppState, id: i64) -> ApiResult<ProcDef> {
    let conn = state.db.lock().await;
    supervisor::get_proc(&conn, id)?.ok_or_else(|| ApiError::NotFound(format!(
        "process {id} not found"
    )))
}

/// Run a sync core op on a blocking thread.
async fn run_blocking<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))?
}

// ── CRUD ────────────────────────────────────────────────────────────

/// GET /api/supervisor — all definitions with live runtime state.
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<ProcInfo>>> {
    let defs = {
        let conn = state.db.lock().await;
        supervisor::list_procs(&conn)?
    };
    let dir = state.config.paths.supervisor.clone();
    let now = state.now_ts();
    let infos = run_blocking(move || {
        Ok(defs
            .into_iter()
            .map(|d| ProcInfo::build(&dir, d, now))
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(Json(infos))
}

/// GET /api/supervisor/{id} — one definition with live runtime state.
pub async fn get_one(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<ProcInfo>> {
    let def = get_def(&state, id).await?;
    let dir = state.config.paths.supervisor.clone();
    let now = state.now_ts();
    let info = run_blocking(move || Ok(ProcInfo::build(&dir, def, now))).await?;
    Ok(Json(info))
}

/// POST /api/supervisor — create a definition.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<ProcReq>,
) -> ApiResult<(StatusCode, Json<ProcInfo>)> {
    let input = supervisor::validate_input(&req)?;
    let now = state.now_ts();
    let def = {
        let conn = state.db.lock().await;
        if supervisor::get_proc_by_name(&conn, &input.name)?.is_some() {
            return Err(ApiError::Conflict(format!(
                "name '{}' already exists",
                input.name
            )));
        }
        supervisor::create_proc(&conn, &input, now)?
    };
    audit::record(
        &state,
        Some(&user.username),
        "POST",
        "/api/supervisor",
        201,
        Some(format!("supervisor create '{}'", input.name)),
    );
    Ok((
        StatusCode::CREATED,
        Json(ProcInfo {
            def,
            state: "stopped",
            sup_pid: None,
            pid: None,
            started_at: None,
            uptime: None,
        }),
    ))
}

/// PUT /api/supervisor/{id} — update a definition (only while stopped).
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<ProcReq>,
) -> ApiResult<Json<ProcInfo>> {
    let input = supervisor::validate_input(&req)?;
    // Serialize with lifecycle ops so the stopped-check cannot race a start.
    let _guard = state.supervisor_lock.lock().await;
    let old = get_def(&state, id).await?;
    let old_name = old.name.clone();
    let dir = state.config.paths.supervisor.clone();
    let st_name = old_name.clone();
    let st = run_blocking(move || Ok(supervisor::status(&dir, &st_name))).await?;
    if st.state != "stopped" {
        return Err(ApiError::Conflict(format!(
            "process '{old_name}' must be stopped before editing"
        )));
    }

    let now = state.now_ts();
    let renamed = old_name != input.name;
    let def = {
        let conn = state.db.lock().await;
        if renamed && supervisor::get_proc_by_name(&conn, &input.name)?.is_some() {
            return Err(ApiError::Conflict(format!(
                "name '{}' already exists",
                input.name
            )));
        }
        supervisor::update_proc(&conn, id, &input, now)?
    };
    if renamed {
        let old = old_name.clone();
        let dir = state.config.paths.supervisor.clone();
        run_blocking(move || Ok(supervisor::remove_runtime_files(&dir, &old))).await?;
    }
    audit::record(
        &state,
        Some(&user.username),
        "PUT",
        &format!("/api/supervisor/{id}"),
        200,
        Some(format!("supervisor update '{}'{}", input.name, if renamed { format!(" (was '{old_name}')") } else { String::new() })),
    );
    Ok(Json(ProcInfo {
        def,
        state: "stopped",
        sup_pid: None,
        pid: None,
        started_at: None,
        uptime: None,
    }))
}

/// DELETE /api/supervisor/{id} — remove a definition (only while stopped)
/// and clean up its runtime files.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let _guard = state.supervisor_lock.lock().await;
    let def = get_def(&state, id).await?;
    let dir = state.config.paths.supervisor.clone();
    let name = def.name.clone();
    let st = run_blocking(move || Ok(supervisor::status(&dir, &name))).await?;
    if st.state != "stopped" {
        return Err(ApiError::Conflict(format!(
            "process '{}' must be stopped before deleting",
            def.name
        )));
    }
    {
        let conn = state.db.lock().await;
        supervisor::delete_proc(&conn, id)?;
    }
    let dir = state.config.paths.supervisor.clone();
    let del_name = def.name.clone();
    run_blocking(move || Ok(supervisor::remove_runtime_files(&dir, &del_name))).await?;
    audit::record(
        &state,
        Some(&user.username),
        "DELETE",
        &format!("/api/supervisor/{id}"),
        200,
        Some(format!("supervisor delete '{}'", def.name)),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Lifecycle ───────────────────────────────────────────────────────

/// POST /api/supervisor/{id}/start
pub async fn start(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<ProcStatus>> {
    let _guard = state.supervisor_lock.lock().await;
    let def = get_def(&state, id).await?;
    let dir = state.config.paths.supervisor.clone();
    let start_def = def.clone();
    run_blocking(move || supervisor::start(&dir, &start_def)).await?;
    let dir = state.config.paths.supervisor.clone();
    let name = def.name.clone();
    let st = run_blocking(move || Ok(supervisor::status(&dir, &name))).await?;
    audit::record(
        &state,
        Some(&user.username),
        "POST",
        &format!("/api/supervisor/{id}/start"),
        200,
        Some(format!("supervisor start '{}'", def.name)),
    );
    Ok(Json(st))
}

/// POST /api/supervisor/{id}/stop
pub async fn stop(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<ProcStatus>> {
    let _guard = state.supervisor_lock.lock().await;
    let def = get_def(&state, id).await?;
    let dir = state.config.paths.supervisor.clone();
    let name = def.name.clone();
    run_blocking(move || supervisor::stop(&dir, &name)).await?;
    let dir = state.config.paths.supervisor.clone();
    let st_name = def.name.clone();
    let st = run_blocking(move || Ok(supervisor::status(&dir, &st_name))).await?;
    audit::record(
        &state,
        Some(&user.username),
        "POST",
        &format!("/api/supervisor/{id}/stop"),
        200,
        Some(format!("supervisor stop '{}'", def.name)),
    );
    Ok(Json(st))
}

/// POST /api/supervisor/{id}/restart — stop then start.
pub async fn restart(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<ProcStatus>> {
    let _guard = state.supervisor_lock.lock().await;
    let def = get_def(&state, id).await?;
    let dir = state.config.paths.supervisor.clone();
    {
        let d = dir.clone();
        let name = def.name.clone();
        run_blocking(move || supervisor::stop(&d, &name)).await?;
    }
    {
        let d = dir.clone();
        let rd = def.clone();
        run_blocking(move || supervisor::start(&d, &rd)).await?;
    }
    let name = def.name.clone();
    let st = run_blocking(move || Ok(supervisor::status(&dir, &name))).await?;
    audit::record(
        &state,
        Some(&user.username),
        "POST",
        &format!("/api/supervisor/{id}/restart"),
        200,
        Some(format!("supervisor restart '{}'", def.name)),
    );
    Ok(Json(st))
}

// ── Logs ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub lines: Option<usize>,
}

/// GET /api/supervisor/{id}/log?lines=200 — tail of the log.
pub async fn read_log(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    Query(q): Query<LogQuery>,
) -> ApiResult<Json<supervisor::LogData>> {
    let def = get_def(&state, id).await?;
    let lines = q.lines.unwrap_or(200).clamp(1, 2000);
    let dir = state.config.paths.supervisor.clone();
    let data = run_blocking(move || supervisor::read_log(&dir, &def, lines)).await?;
    Ok(Json(data))
}

/// DELETE /api/supervisor/{id}/log — clear the log.
pub async fn clear_log(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let def = get_def(&state, id).await?;
    let name = def.name.clone();
    let dir = state.config.paths.supervisor.clone();
    run_blocking(move || supervisor::clear_log(&dir, &def)).await?;
    audit::record(
        &state,
        Some(&user.username),
        "DELETE",
        &format!("/api/supervisor/{id}/log"),
        200,
        Some(format!("supervisor clear log '{}'", name)),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}
