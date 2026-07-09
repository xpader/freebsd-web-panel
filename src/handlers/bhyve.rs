//! Bhyve virtual machine management — HTTP handlers.
//!
//! Wraps the vm-bhyve CLI via `crate::bhyve`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::bhyve;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub running: bool,
}

/// GET /api/bhyve/vms — list all VMs (or running-only if ?running=true).
pub async fn list_vms(Query(q): Query<ListQuery>) -> ApiResult<Json<Vec<bhyve::VmSummary>>> {
    let vms = bhyve::list_vms(q.running).map_err(ApiError::Command)?;
    Ok(Json(vms))
}

/// POST /api/bhyve/vms — create a new VM.
#[derive(Debug, Deserialize)]
pub struct CreateVmBody {
    pub name: String,
    pub template: String,
    pub datastore: Option<String>,
    pub size: Option<String>,
    pub cpu: Option<u32>,
    pub memory: Option<String>,
}

pub async fn create_vm(
    State(state): State<AppState>,
    Json(body): Json<CreateVmBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    validate_vm_name(&body.name)?;

    if body.template.is_empty() {
        return Err(ApiError::BadRequest("template is required".into()));
    }

    // Execute vm create inside spawn_blocking (it does ZFS + disk I/O).
    let name = body.name.clone();
    let template = body.template.clone();
    let datastore = body.datastore.clone();
    let size = body.size.clone();
    let cpu = body.cpu;
    let memory = body.memory.clone();

    let result = tokio::task::spawn_blocking(move || {
        bhyve::create_vm(
            &name,
            &template,
            datastore.as_deref(),
            size.as_deref(),
            cpu,
            memory.as_deref(),
        )
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(ApiError::Command)?;

    let _ = result;

    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/bhyve/vms",
        201,
        Some(format!("created vm {}", body.name)),
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "name": body.name })),
    ))
}

/// GET /api/bhyve/vms/{name} — VM detail (vm info + .conf).
pub async fn vm_detail(
    Path(name): Path<String>,
) -> ApiResult<Json<bhyve::VmDetail>> {
    validate_vm_name(&name)?;
    let name_clone = name.clone();
    let detail = tokio::task::spawn_blocking(move || bhyve::get_vm_info(&name_clone))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;
    Ok(Json(detail))
}

/// POST /api/bhyve/vms/{name}/start
pub async fn vm_start(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_vm_name(&name)?;
    let n = name.clone();
    tokio::task::spawn_blocking(move || bhyve::start_vm(&n))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        &format!("/api/bhyve/vms/{}/start", name),
        200,
        Some(format!("started vm {}", name)),
    );
    Ok(StatusCode::OK)
}

/// POST /api/bhyve/vms/{name}/stop
pub async fn vm_stop(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_vm_name(&name)?;
    let n = name.clone();
    tokio::task::spawn_blocking(move || bhyve::stop_vm(&n))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        &format!("/api/bhyve/vms/{}/stop", name),
        200,
        Some(format!("stopped vm {}", name)),
    );
    Ok(StatusCode::OK)
}

/// GET /api/bhyve/images — list vm-bhyve images.
pub async fn list_images() -> ApiResult<Json<Vec<bhyve::VmImage>>> {
    let images = bhyve::list_images().map_err(ApiError::Command)?;
    Ok(Json(images))
}

/// GET /api/bhyve/switches — list virtual switches.
pub async fn list_switches() -> ApiResult<Json<Vec<bhyve::VmSwitch>>> {
    let switches = bhyve::list_switches().map_err(ApiError::Command)?;
    Ok(Json(switches))
}

/// GET /api/bhyve/datastores — list datastores.
pub async fn list_datastores() -> ApiResult<Json<Vec<bhyve::VmDatastore>>> {
    let ds = bhyve::list_datastores().map_err(ApiError::Command)?;
    Ok(Json(ds))
}

/// GET /api/bhyve/templates — list available templates.
pub async fn list_templates() -> ApiResult<Json<Vec<String>>> {
    let templates = bhyve::list_templates().map_err(ApiError::Command)?;
    Ok(Json(templates))
}

/// GET /api/bhyve/isos — list available ISO images.
pub async fn list_isos() -> ApiResult<Json<Vec<bhyve::IsoImage>>> {
    let isos = bhyve::list_isos().map_err(ApiError::Command)?;
    Ok(Json(isos))
}

/// Validate VM name: must match vm-bhyve rules (lowercase, [a-z0-9._-]).
fn validate_vm_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("invalid VM name".into()));
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-');
    if !valid {
        return Err(ApiError::BadRequest(
            "VM name may only contain lowercase letters, digits, dots, underscores and dashes"
                .into(),
        ));
    }
    // vm-bhyve requires the first and last chars to be alphanumeric.
    if !name.chars().next().unwrap().is_ascii_alphanumeric()
        || !name.chars().last().unwrap().is_ascii_alphanumeric()
    {
        return Err(ApiError::BadRequest(
            "VM name must start and end with a letter or digit".into(),
        ));
    }
    Ok(())
}
