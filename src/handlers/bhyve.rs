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

/// GET /api/bhyve/vms/{name}/disk-resources — list available disk files and ZVOLs.
pub async fn disk_resources(
    Path(name): Path<String>,
) -> ApiResult<Json<bhyve::DiskResources>> {
    validate_vm_name(&name)?;
    let name_clone = name.clone();
    let resources = tokio::task::spawn_blocking(move || bhyve::list_disk_resources(&name_clone))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;
    Ok(Json(resources))
}

/// PUT /api/bhyve/vms/{name} — replace VM configuration key-values.
/// The body is split into semantic sections; the backend merges them into one
/// key-value map before writing.
#[derive(Debug, Deserialize)]
pub struct UpdateVmConfigBody {
    #[serde(default)]
    pub config: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub advance: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub graphics: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub other_devices: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub auto_start: Option<bool>,
}

pub async fn update_vm_config(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<UpdateVmConfigBody>,
) -> ApiResult<StatusCode> {
    validate_vm_name(&name)?;

    let mut merged = std::collections::BTreeMap::new();
    for section in [&body.config, &body.advance, &body.graphics, &body.other_devices] {
        for (k, v) in section {
            merged.insert(k.clone(), v.clone());
        }
    }
    let has_config = !merged.is_empty();
    if !has_config && body.auto_start.is_none() {
        return Err(ApiError::BadRequest(
            "at least one configuration section must not be empty".into(),
        ));
    }
    if has_config {
        validate_vm_config(&merged)?;
        let vm_name = name.clone();
        tokio::task::spawn_blocking(move || bhyve::update_vm_config(&vm_name, &merged))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
            .map_err(ApiError::Command)?;
    }

    let mut audit_msgs = vec![];
    if has_config {
        audit_msgs.push(format!("updated VM configuration {name}"));
    }
    if let Some(want_auto) = body.auto_start {
        let n = name.clone();
        tokio::task::spawn_blocking(move || bhyve::set_vm_auto_start(&n, want_auto))
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
            .map_err(ApiError::Command)?;
        audit_msgs.push(format!(
            "{} auto-start for vm {}",
            if want_auto { "enabled" } else { "disabled" },
            name
        ));
    }

    crate::audit::record(
        &state,
        None,
        "PUT",
        &format!("/api/bhyve/vms/{name}"),
        200,
        Some(audit_msgs.join("; ")),
    );
    Ok(StatusCode::OK)
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

/// POST /api/bhyve/vms/{name}/disks — create and attach a new disk.
#[derive(Debug, Deserialize)]
pub struct AddDiskBody {
    pub disk_type: String,
    pub size: String,
}

pub async fn add_disk(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<AddDiskBody>,
) -> ApiResult<StatusCode> {
    validate_vm_name(&name)?;
    let n = name.clone();
    let dt = body.disk_type.clone();
    let sz = body.size.clone();
    tokio::task::spawn_blocking(move || bhyve::add_disk(&n, &dt, &sz))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        &format!("/api/bhyve/vms/{}/disks", name),
        200,
        Some(format!("created disk (type={}, size={}) for vm {}", body.disk_type, body.size, name)),
    );
    Ok(StatusCode::OK)
}

/// DELETE /api/bhyve/vms/{name}/disks/{index} — remove a disk from config (does not delete data).
pub async fn delete_disk(
    State(state): State<AppState>,
    Path((name, index)): Path<(String, u32)>,
) -> ApiResult<StatusCode> {
    validate_vm_name(&name)?;
    let n = name.clone();
    tokio::task::spawn_blocking(move || bhyve::delete_device(&n, "disk", index))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "DELETE",
        &format!("/api/bhyve/vms/{}/disks/{}", name, index),
        200,
        Some(format!("removed disk {} from vm {}", index, name)),
    );
    Ok(StatusCode::OK)
}

/// DELETE /api/bhyve/vms/{name}/networks/{index} — remove a network interface from config.
pub async fn delete_network(
    State(state): State<AppState>,
    Path((name, index)): Path<(String, u32)>,
) -> ApiResult<StatusCode> {
    validate_vm_name(&name)?;
    let n = name.clone();
    tokio::task::spawn_blocking(move || bhyve::delete_device(&n, "network", index))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "DELETE",
        &format!("/api/bhyve/vms/{}/networks/{}", name, index),
        200,
        Some(format!("removed network {} from vm {}", index, name)),
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

/// GET /api/bhyve/switches/{name} — virtual switch detail.
pub async fn switch_detail(
    Path(name): Path<String>,
) -> ApiResult<Json<bhyve::VmSwitchDetail>> {
    validate_switch_name(&name)?;
    let switch_name = name.clone();
    let detail = tokio::task::spawn_blocking(move || bhyve::get_switch_info(&switch_name))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;
    Ok(Json(detail))
}

/// POST /api/bhyve/switches — create a virtual switch.
#[derive(Debug, Deserialize)]
pub struct CreateSwitchBody {
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub iface: Option<String>,
    pub vlan: Option<u16>,
    pub bridge: Option<String>,
    pub address: Option<String>,
    pub mtu: Option<u16>,
    #[serde(default)]
    pub private: bool,
}

pub async fn create_switch(
    State(state): State<AppState>,
    Json(body): Json<CreateSwitchBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    validate_switch_name(&body.name)?;
    validate_switch_create(&body)?;

    let name = body.name.clone();
    let typ = body.typ.clone();
    let iface = body.iface.clone();
    let vlan = body.vlan;
    let bridge = body.bridge.clone();
    let address = body.address.clone();
    let mtu = body.mtu;
    let private = body.private;
    tokio::task::spawn_blocking(move || {
        bhyve::create_switch(
            &name,
            &typ,
            iface.as_deref(),
            vlan,
            bridge.as_deref(),
            address.as_deref(),
            mtu,
            private,
        )
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/bhyve/switches",
        201,
        Some(format!("created {} switch {}", body.typ, body.name)),
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "name": body.name })),
    ))
}

/// DELETE /api/bhyve/switches/{name} — destroy a virtual switch.
pub async fn delete_switch(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_switch_name(&name)?;
    let switch_name = name.clone();
    tokio::task::spawn_blocking(move || bhyve::destroy_switch(&switch_name))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "DELETE",
        &format!("/api/bhyve/switches/{name}"),
        200,
        Some(format!("destroyed switch {name}")),
    );
    Ok(StatusCode::OK)
}

/// GET /api/bhyve/status — check vm-bhyve installation and configuration status.
pub async fn status() -> ApiResult<Json<bhyve::BhyveStatus>> {
    let s = tokio::task::spawn_blocking(bhyve::check_status)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(s))
}

/// POST /api/bhyve/init — initialize vm-bhyve.
/// Body: { "spec": "zfs:zroot/vm" } or { "spec": "/home/vm" }
#[derive(Debug, Deserialize)]
pub struct InitBody {
    pub spec: String,
}

pub async fn init(
    State(state): State<AppState>,
    Json(body): Json<InitBody>,
) -> ApiResult<Json<Vec<String>>> {
    validate_init_spec(&body.spec)?;

    let spec = body.spec.clone();
    let steps = tokio::task::spawn_blocking(move || bhyve::init_bhyve(&spec))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/bhyve/init",
        200,
        Some(format!("vm-bhyve initialized (vm_dir={})", body.spec)),
    );

    Ok(Json(steps))
}

/// GET /api/bhyve/datastores — list datastores.
pub async fn list_datastores() -> ApiResult<Json<Vec<bhyve::VmDatastore>>> {
    let ds = bhyve::list_datastores().map_err(ApiError::Command)?;
    Ok(Json(ds))
}

/// POST /api/bhyve/datastores — add a new datastore.
#[derive(Debug, Deserialize)]
pub struct CreateDatastoreBody {
    pub name: String,
    pub spec: String,
}

pub async fn create_datastore(
    State(state): State<AppState>,
    Json(body): Json<CreateDatastoreBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    validate_datastore_name(&body.name)?;

    if body.spec.is_empty() {
        return Err(ApiError::BadRequest("spec is required".into()));
    }

    let name = body.name.clone();
    let spec = body.spec.clone();
    tokio::task::spawn_blocking(move || bhyve::add_datastore(&name, &spec))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/bhyve/datastores",
        201,
        Some(format!("added datastore {} ({})", body.name, body.spec)),
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "name": body.name })),
    ))
}

/// DELETE /api/bhyve/datastores/{name} — remove a datastore.
pub async fn delete_datastore(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    if name == "default" {
        return Err(ApiError::BadRequest(
            "cannot remove the default datastore".into(),
        ));
    }
    validate_datastore_name(&name)?;
    let n = name.clone();
    tokio::task::spawn_blocking(move || bhyve::remove_datastore(&n))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
        .map_err(ApiError::Command)?;

    crate::audit::record(
        &state,
        None,
        "DELETE",
        &format!("/api/bhyve/datastores/{}", name),
        200,
        Some(format!("removed datastore {}", name)),
    );
    Ok(StatusCode::OK)
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

fn validate_vm_config(config: &std::collections::BTreeMap<String, String>) -> ApiResult<()> {
    if config.is_empty() {
        return Err(ApiError::BadRequest("VM configuration must not be empty".into()));
    }
    if config.len() > 256 {
        return Err(ApiError::BadRequest("VM configuration has too many entries".into()));
    }
    for (key, value) in config {
        if key.is_empty()
            || key.len() > 128
            || !key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(ApiError::BadRequest(format!("invalid VM configuration key: {key}")));
        }
        if value.len() > 4096 || value.contains('\0') || value.contains('\n') || value.contains('\r') {
            return Err(ApiError::BadRequest(format!("invalid value for VM configuration key: {key}")));
        }
    }
    Ok(())
}

/// Validate datastore name: same rules as VM names — lowercase
/// alphanumeric with `.`/`_`/`-`, must start/end with alphanumeric.
fn validate_datastore_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("invalid datastore name".into()));
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-');
    if !valid {
        return Err(ApiError::BadRequest(
            "datastore name may only contain lowercase letters, digits, dots, underscores and dashes"
                .into(),
        ));
    }
    if !name.chars().next().unwrap().is_ascii_alphanumeric()
        || !name.chars().last().unwrap().is_ascii_alphanumeric()
    {
        return Err(ApiError::BadRequest(
            "datastore name must start and end with a letter or digit".into(),
        ));
    }
    Ok(())
}

fn validate_switch_name(name: &str) -> ApiResult<()> {
    validate_datastore_name(name).map_err(|_| {
        ApiError::BadRequest(
            "switch name may only contain lowercase letters, digits, dots, underscores and dashes, and must start and end with a letter or digit".into(),
        )
    })
}

fn validate_switch_create(body: &CreateSwitchBody) -> ApiResult<()> {
    match body.typ.as_str() {
        "standard" | "manual" | "netgraph" | "vale" | "vxlan" => {}
        _ => return Err(ApiError::BadRequest("invalid switch type".into())),
    }
    if body.typ == "manual" && body.bridge.as_deref().unwrap_or_default().is_empty() {
        return Err(ApiError::BadRequest("manual switches require a bridge".into()));
    }
    if body.typ == "vxlan" && (body.iface.as_deref().unwrap_or_default().is_empty() || body.vlan.is_none()) {
        return Err(ApiError::BadRequest("vxlan switches require an interface and VLAN ID".into()));
    }
    if body.vlan.is_some_and(|value| value >= 4095) {
        return Err(ApiError::BadRequest("VLAN ID must be between 0 and 4094".into()));
    }
    if body.mtu.is_some_and(|value| !(100..=9000).contains(&value)) {
        return Err(ApiError::BadRequest("MTU must be between 100 and 9000".into()));
    }
    if let Some(address) = &body.address {
        let (ip, prefix) = address
            .split_once('/')
            .ok_or_else(|| ApiError::BadRequest("address must use CIDR notation".into()))?;
        if ip.parse::<std::net::Ipv4Addr>().is_err()
            || prefix.parse::<u8>().map_or(true, |value| value > 32)
        {
            return Err(ApiError::BadRequest("address must use IPv4 CIDR notation".into()));
        }
    }
    Ok(())
}

/// Validate vm_dir spec for initialization.
/// Must be either an absolute path (`/path/to/dir`) or `zfs:pool/dataset`.
fn validate_init_spec(spec: &str) -> ApiResult<()> {
    if spec.is_empty() || spec.len() > 256 {
        return Err(ApiError::BadRequest("invalid vm_dir spec".into()));
    }
    if spec.starts_with("zfs:") {
        let dataset = &spec[4..];
        if dataset.is_empty() {
            return Err(ApiError::BadRequest("ZFS dataset name is empty".into()));
        }
        let valid = dataset
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' || c == '.');
        if !valid {
            return Err(ApiError::BadRequest(
                "ZFS dataset name may only contain letters, digits, /, _, - and .".into(),
            ));
        }
    } else {
        if !spec.starts_with('/') {
            return Err(ApiError::BadRequest(
                "directory path must be absolute (start with /)".into(),
            ));
        }
        if spec.contains('\0') {
            return Err(ApiError::BadRequest("invalid path".into()));
        }
    }
    Ok(())
}
