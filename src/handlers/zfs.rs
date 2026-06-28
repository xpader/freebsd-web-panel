//! ZFS management — pools, datasets, snapshots.
//!
//! All commands use `zfs`/`zpool` with `-H -p` machine-readable output.
//! Inputs are validated against a strict pattern before being passed as
//! command arguments (no shell interpolation).

use std::collections::HashMap;
use std::process::Command;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const ZFS: &str = "/sbin/zfs";
const ZPOOL: &str = "/sbin/zpool";

/// Validate a dataset/pool/snapshot name. ZFS names allow alphanumerics,
/// '/', '_', '-', '.', ':' (for snapshots '@') and no leading dot.
fn validate_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 256 {
        return Err(ApiError::BadRequest("invalid name length".into()));
    }
    let re = Regex::new(r"^[a-zA-Z0-9@/_:\-\.]+$").unwrap();
    if !re.is_match(name) || name.starts_with('.') || name.contains("..") {
        return Err(ApiError::BadRequest("invalid name".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct NameQuery {
    pub name: String,
}

fn run(cmd: &str, args: &[&str]) -> ApiResult<String> {
    let output = Command::new(cmd).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::Internal(if stderr.is_empty() {
            format!("{cmd} failed").into()
        } else {
            stderr
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ===== Zpool =====

#[derive(Debug, Serialize)]
pub struct ZpoolInfo {
    pub name: String,
    pub size: u64,
    pub allocated: u64,
    pub free: u64,
    pub fragmentation_pct: f32,
    pub capacity_pct: f32,
    pub dedup: f32,
    pub health: String,
    pub scan: Option<String>,
    pub vdevs: Vec<Vdev>,
    pub error_text: String,
}

#[derive(Debug, Serialize)]
pub struct Vdev {
    pub name: String,
    pub state: String,
    pub read_errors: u64,
    pub write_errors: u64,
    pub checksum_errors: u64,
    pub indent: usize,
    pub children: Vec<Vdev>,
}

pub async fn pool_list() -> ApiResult<Json<Vec<ZpoolSummary>>> {
    let raw = run(ZPOOL, &["list", "-H", "-p"])?;
    let pools: Vec<ZpoolSummary> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 10 {
                return None;
            }
            let p = |i: usize| -> u64 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
            let pf = |i: usize| -> f32 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
            Some(ZpoolSummary {
                name: cols[0].into(),
                size: p(1),
                allocated: p(2),
                free: p(3),
                fragmentation_pct: pf(6),
                capacity_pct: pf(7),
                dedup: pf(8),
                health: cols.get(9).map(|s| (*s).to_string()).unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(pools))
}

#[derive(Debug, Serialize)]
pub struct ZpoolSummary {
    pub name: String,
    pub size: u64,
    pub allocated: u64,
    pub free: u64,
    pub fragmentation_pct: f32,
    pub capacity_pct: f32,
    pub dedup: f32,
    pub health: String,
}

pub async fn pool_status(Path(name): Path<String>) -> ApiResult<Json<ZpoolInfo>> {
    validate_name(&name)?;
    let mut info = parse_zpool_status(&run(ZPOOL, &["status", &name])?, &name);
    // Enrich with size/alloc/free/frag/cap/dedup from `zpool list`.
    let list_raw = run(ZPOOL, &["list", "-H", "-p", &name])?;
    if let Some(line) = list_raw.lines().next() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 10 {
            let p = |i: usize| -> u64 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
            let pf = |i: usize| -> f32 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
            info.size = p(1);
            info.allocated = p(2);
            info.free = p(3);
            info.fragmentation_pct = pf(6);
            info.capacity_pct = pf(7);
            info.dedup = pf(8);
        }
    }
    Ok(Json(info))
}

fn parse_zpool_status(raw: &str, pool_name: &str) -> ZpoolInfo {
    let mut info = ZpoolInfo {
        name: pool_name.into(),
        size: 0,
        allocated: 0,
        free: 0,
        fragmentation_pct: 0.0,
        capacity_pct: 0.0,
        dedup: 1.0,
        health: "UNKNOWN".into(),
        scan: None,
        vdevs: vec![],
        error_text: String::new(),
    };

    let mut in_config = false;
    let mut flat_vdevs: Vec<(usize, Vdev)> = vec![];

    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with("state:") {
            info.health = t.trim_start_matches("state:").trim().into();
        } else if t.starts_with("scan:") {
            info.scan = Some(t.trim_start_matches("scan:").trim().into());
        } else if t.starts_with("errors:") {
            info.error_text = t.trim_start_matches("errors:").trim().into();
        } else if t.contains("config:") {
            in_config = true;
            continue;
        }

        if in_config && line.starts_with('\t') {
            // Vdev line: detect indent from original line.
            let indent = line.len() - line.trim_start().len();
            let cols: Vec<&str> = t.split_whitespace().collect();
            if cols.len() < 5 {
                continue;
            }
            // Skip header line.
            if cols[0] == "NAME" {
                continue;
            }
            // Skip errors summary line.
            if cols[0].starts_with("errors") {
                in_config = false;
                continue;
            }
            let v = Vdev {
                name: cols[0].into(),
                state: cols[1].into(),
                read_errors: cols[2].parse().unwrap_or(0),
                write_errors: cols[3].parse().unwrap_or(0),
                checksum_errors: cols[4].parse().unwrap_or(0),
                indent,
                children: vec![],
            };
            flat_vdevs.push((indent, v));
        }
    }

    // Build tree from indent levels.
    if let Some((_, pool_vdev)) = flat_vdevs.first().cloned() {
        info.name = pool_vdev.name.clone();
        info.health = pool_vdev.state.clone();
        let mut root_children: Vec<Vdev> = vec![];
        if flat_vdevs.len() > 1 {
            let rest: Vec<(usize, Vdev)> = flat_vdevs[1..].iter().cloned().collect();
            build_vdev_tree(&rest, &mut root_children);
        }
        info.vdevs = root_children;
    }

    info
}

fn build_vdev_tree(items: &[(usize, Vdev)], out: &mut Vec<Vdev>) {
    if items.is_empty() {
        return;
    }
    let base_indent = items[0].0;
    let mut i = 0;
    while i < items.len() {
        let (indent, mut v) = (items[i].0, items[i].1.clone());
        if indent != base_indent {
            i += 1;
            continue;
        }
        // Collect children: all following items with deeper indent until same/less.
        let mut j = i + 1;
        let mut children_items: Vec<(usize, Vdev)> = vec![];
        while j < items.len() && items[j].0 > base_indent {
            children_items.push(items[j].clone());
            j += 1;
        }
        if !children_items.is_empty() {
            build_vdev_tree(&children_items, &mut v.children);
        }
        out.push(v);
        i = j;
    }
}

// ===== Datasets =====

#[derive(Debug, Serialize)]
pub struct Dataset {
    pub name: String,
    pub used: u64,
    pub available: u64,
    pub referenced: u64,
    pub mountpoint: String,
    pub typ: String,
    pub compression: String,
    pub children: Vec<Dataset>,
}

pub async fn dataset_list() -> ApiResult<Json<Vec<Dataset>>> {
    let raw = run(
        ZFS,
        &["list", "-H", "-p", "-o", "name,used,avail,refer,mountpoint,type,compression"],
    )?;
    let flat: Vec<Dataset> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 7 {
                return None;
            }
            let p = |i: usize| -> u64 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
            Some(Dataset {
                name: cols[0].into(),
                used: p(1),
                available: p(2),
                referenced: p(3),
                mountpoint: cols[4].into(),
                typ: cols[5].into(),
                compression: cols[6].into(),
                children: vec![],
            })
        })
        .collect();
    Ok(Json(build_dataset_tree(flat)))
}

fn build_dataset_tree(flat: Vec<Dataset>) -> Vec<Dataset> {
    // Group by depth (number of '/' segments), build tree top-down.
    use std::collections::BTreeMap;
    let mut by_name: HashMap<String, Dataset> = HashMap::new();
    let mut parent_map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for ds in flat {
        let name = ds.name.clone();
        if let Some(parent) = name.rsplitn(2, '/').nth(1) {
            parent_map.entry(parent.into()).or_default().push(name.clone());
        } else {
            // Top-level pool — insert as root key.
            parent_map.entry(String::new()).or_default().push(name.clone());
        }
        by_name.insert(name, ds);
    }

    fn populate(
        name: &str,
        by_name: &mut HashMap<String, Dataset>,
        parent_map: &BTreeMap<String, Vec<String>>,
    ) -> Dataset {
        let mut ds = by_name.remove(name).unwrap_or(Dataset {
            name: name.into(),
            used: 0,
            available: 0,
            referenced: 0,
            mountpoint: String::new(),
            typ: String::new(),
            compression: String::new(),
            children: vec![],
        });
        if let Some(children) = parent_map.get(name) {
            for child in children {
                ds.children.push(populate(child, by_name, parent_map));
            }
        }
        ds
    }

    let roots = parent_map.get("").cloned().unwrap_or_default();
    roots
        .into_iter()
        .map(|r| populate(&r, &mut by_name, &parent_map))
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct DatasetCreateBody {
    pub name: String,
    pub properties: Option<HashMap<String, String>>,
}

pub async fn dataset_create(
    State(state): State<AppState>,
    body: axum::Json<DatasetCreateBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    validate_name(&body.name)?;
    let mut args: Vec<String> = vec!["create".into()];
    if let Some(props) = &body.properties {
        for (k, v) in props {
            validate_prop_key(k)?;
            args.push("-o".into());
            args.push(format!("{k}={v}"));
        }
    }
    args.push(body.name.clone());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(ZFS, &arg_refs)?;
    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/zfs/datasets",
        201,
        Some(format!("created dataset {}", body.name)),
    );
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"name": body.name})),
    ))
}

pub async fn dataset_destroy(
    State(state): State<AppState>,
    Query(q): Query<NameQuery>,
) -> ApiResult<StatusCode> {
    let name = &q.name;
    validate_name(name)?;
    run(ZFS, &["destroy", "-r", name])?;
    crate::audit::record(
        &state,
        None,
        "DELETE",
        "/api/zfs/dataset/destroy",
        200,
        Some(format!("destroyed dataset {}", name)),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct DatasetSetBody {
    pub properties: HashMap<String, String>,
}

pub async fn dataset_set(
    State(state): State<AppState>,
    Query(q): Query<NameQuery>,
    body: axum::Json<DatasetSetBody>,
) -> ApiResult<StatusCode> {
    let name = &q.name;
    validate_name(name)?;
    for (k, v) in &body.properties {
        validate_prop_key(k)?;
        run(ZFS, &["set", &format!("{k}={v}"), name])?;
    }
    crate::audit::record(
        &state, None, "PUT", "/api/zfs/dataset/properties", 200,
        Some(format!("set properties on {}", name)),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn dataset_properties(Query(q): Query<NameQuery>) -> ApiResult<Json<Vec<Property>>> {
    let name = &q.name;
    validate_name(name)?;
    let raw = run(ZFS, &["get", "-H", "-p", "-o", "property,value,source", "all", name])?;
    let props: Vec<Property> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 3 {
                return None;
            }
            Some(Property {
                name: cols[0].into(),
                value: cols[1].into(),
                source: cols[2].into(),
            })
        })
        .collect();
    Ok(Json(props))
}

#[derive(Debug, Serialize)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub source: String,
}

fn validate_prop_key(k: &str) -> ApiResult<()> {
    let re = Regex::new(r"^[a-zA-Z0-9_:\-\.]+$").unwrap();
    if k.is_empty() || k.len() > 128 || !re.is_match(k) {
        return Err(ApiError::BadRequest("invalid property name".into()));
    }
    Ok(())
}

// ===== Snapshots =====

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub name: String,
    pub dataset: String,
    pub snap_name: String,
    pub used: u64,
    pub referenced: u64,
    pub creation: i64,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotQuery {
    pub dataset: Option<String>,
}

pub async fn snapshot_list(
    Query(q): Query<SnapshotQuery>,
) -> ApiResult<Json<Vec<Snapshot>>> {
    let mut args = vec!["list", "-t", "snapshot", "-H", "-p", "-o", "name,used,refer,creation"];
    let mut owned_args: Vec<String> = vec![];
    if let Some(ref ds) = q.dataset {
        validate_name(ds)?;
        owned_args.push(ds.clone());
    }
    let arg_refs: Vec<&str> = args
        .iter()
        .copied()
        .chain(owned_args.iter().map(|s| s.as_str()))
        .collect();
    let raw = run(ZFS, &arg_refs)?;
    let snaps: Vec<Snapshot> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 4 {
                return None;
            }
            let full = cols[0];
            let (dataset, snap_name) = full.split_once('@')?;
            let p = |i: usize| -> u64 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
            let creation: i64 = cols.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            Some(Snapshot {
                name: full.into(),
                dataset: dataset.into(),
                snap_name: snap_name.into(),
                used: p(1),
                referenced: p(2),
                creation,
            })
        })
        .collect();
    Ok(Json(snaps))
}

#[derive(Debug, Deserialize)]
pub struct SnapshotCreateBody {
    pub dataset: String,
    pub name: String,
}

pub async fn snapshot_create(
    State(state): State<AppState>,
    body: axum::Json<SnapshotCreateBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    validate_name(&body.dataset)?;
    let snap_name = body.name.trim();
    if snap_name.is_empty() || snap_name.contains('@') || snap_name.contains('/') {
        return Err(ApiError::BadRequest("invalid snapshot name".into()));
    }
    let full = format!("{}@{}", body.dataset, snap_name);
    run(ZFS, &["snapshot", &full])?;
    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/zfs/snapshots",
        201,
        Some(format!("created snapshot {full}")),
    );
    Ok((StatusCode::CREATED, Json(serde_json::json!({"name": full}))))
}

pub async fn snapshot_destroy(
    State(state): State<AppState>,
    Query(q): Query<NameQuery>,
) -> ApiResult<StatusCode> {
    let full = &q.name;
    validate_name(full)?;
    if !full.contains('@') {
        return Err(ApiError::BadRequest("not a snapshot name".into()));
    }
    run(ZFS, &["destroy", full])?;
    crate::audit::record(
        &state, None, "DELETE", "/api/zfs/snapshot/destroy", 200,
        Some(format!("destroyed snapshot {full}")),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct SnapshotRollbackBody {
    /// Require explicit confirmation.
    pub confirm: bool,
}

pub async fn snapshot_rollback(
    State(state): State<AppState>,
    Query(q): Query<NameQuery>,
    body: axum::Json<SnapshotRollbackBody>,
) -> ApiResult<StatusCode> {
    let full = &q.name;
    validate_name(full)?;
    if !full.contains('@') {
        return Err(ApiError::BadRequest("not a snapshot name".into()));
    }
    if !body.confirm {
        return Err(ApiError::BadRequest("confirm=true required for rollback".into()));
    }
    run(ZFS, &["rollback", "-r", full])?;
    crate::audit::record(
        &state, None, "POST", "/api/zfs/snapshot/rollback", 200,
        Some(format!("rolled back to {full}")),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pool_scrub(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    run(ZPOOL, &["scrub", &name])?;
    crate::audit::record(
        &state,
        None,
        "POST",
        &format!("/api/zfs/pools/{}/scrub", name),
        200,
        Some(format!("scrub started on {name}")),
    );
    Ok(StatusCode::OK)
}

pub async fn pool_scrub_stop(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    run(ZPOOL, &["scrub", "-s", &name])?;
    crate::audit::record(
        &state,
        None,
        "POST",
        &format!("/api/zfs/pools/{}/scrub/stop", name),
        200,
        Some(format!("scrub stopped on {name}")),
    );
    Ok(StatusCode::OK)
}

impl Clone for Vdev {
    fn clone(&self) -> Self {
        Vdev {
            name: self.name.clone(),
            state: self.state.clone(),
            read_errors: self.read_errors,
            write_errors: self.write_errors,
            checksum_errors: self.checksum_errors,
            indent: self.indent,
            children: self.children.clone(),
        }
    }
}
