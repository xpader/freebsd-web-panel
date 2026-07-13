//! ZFS management — pools, datasets, snapshots.
//!
//! All commands use `zfs`/`zpool` with `-H -p` machine-readable output.
//! Inputs are validated against a strict pattern before being passed as
//! command arguments (no shell interpolation).

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::cmd;
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

/// Validate a mountpoint path: must start with '/', and contain no
/// null bytes, newlines, or shell metacharacters.
fn validate_mountpoint(mp: &str) -> ApiResult<()> {
    if mp.is_empty() || !mp.starts_with('/') {
        return Err(ApiError::BadRequest("mountpoint must be an absolute path".into()));
    }
    if mp.contains('\0') || mp.contains('\n') || mp.contains('\r') {
        return Err(ApiError::BadRequest("mountpoint contains invalid characters".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct NameQuery {
    pub name: String,
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
    pub expand: Option<String>,
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
    let raw = cmd::run(ZPOOL, &["list", "-H", "-p"]).await?;
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
    let mut info = parse_zpool_status(&cmd::run(ZPOOL, &["status", &name]).await?, &name);
    // Enrich with size/alloc/free/frag/cap/dedup from `zpool list`.
    let list_raw = cmd::run(ZPOOL, &["list", "-H", "-p", &name]).await?;
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

/// Flush a multi-line block (scan or expand) into the corresponding ZpoolInfo field.
fn flush_block(kind: Option<BlockKind>, lines: &mut Vec<String>, info: &mut ZpoolInfo) {
    if lines.is_empty() {
        return;
    }
    match kind {
        Some(BlockKind::Scan) => info.scan = Some(lines.join("\n")),
        Some(BlockKind::Expand) => info.expand = Some(lines.join("\n")),
        None => {}
    }
    lines.clear();
}

enum BlockKind { Scan, Expand }

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
        expand: None,
        vdevs: vec![],
        error_text: String::new(),
    };

    let mut in_config = false;
    let mut flat_vdevs: Vec<(usize, Vdev)> = vec![];

    // Track multi-line blocks (scan/expand). Their first line is space-indented,
    // continuation lines (progress/speed/ETA) are TAB-indented — same as vdev
    // config lines, but appear before `config:`.
    // We use a single tracker: `in_block` + `block_target` points to which field
    // to flush to (scan or expand).
    let mut in_block: Option<BlockKind> = None;
    let mut block_lines: Vec<String> = vec![];

    for line in raw.lines() {
        let t = line.trim();

        // Block continuation: tab-indented line while in_block (before config:).
        if let Some(_) = in_block {
            if line.starts_with('\t') {
                block_lines.push(t.to_string());
                continue;
            }
            // Non-tab line ends the block — flush.
            flush_block(in_block.take(), &mut block_lines, &mut info);
        }

        if t.starts_with("state:") {
            info.health = t.trim_start_matches("state:").trim().into();
        } else if t.starts_with("scan:") {
            in_block = Some(BlockKind::Scan);
            block_lines.push(t.trim_start_matches("scan:").trim().to_string());
        } else if t.starts_with("expand:") {
            in_block = Some(BlockKind::Expand);
            block_lines.push(t.trim_start_matches("expand:").trim().to_string());
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

    // Flush any remaining block (e.g. scan/expand at end of output).
    flush_block(in_block.take(), &mut block_lines, &mut info);

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
    pub origin: Option<String>,
    pub children: Vec<Dataset>,
}

pub async fn dataset_list() -> ApiResult<Json<Vec<Dataset>>> {
    let raw = cmd::run(
        ZFS,
        &["list", "-H", "-p", "-o", "name,used,avail,refer,mountpoint,type,compression,origin"],
    )
    .await?;
    let flat: Vec<Dataset> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 7 {
                return None;
            }
            let p = |i: usize| -> u64 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
            let origin = cols.get(7).filter(|s| **s != "-" && !s.is_empty()).map(|s| s.to_string());
            Some(Dataset {
                name: cols[0].into(),
                used: p(1),
                available: p(2),
                referenced: p(3),
                mountpoint: cols[4].into(),
                typ: cols[5].into(),
                compression: cols[6].into(),
                origin,
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
            origin: None,
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
    cmd::run(ZFS, &arg_refs).await?;
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
    cmd::run(ZFS, &["destroy", "-r", name]).await?;
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
        cmd::run(ZFS, &["set", &format!("{k}={v}"), name]).await?;
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
    let raw = cmd::run(ZFS, &["get", "-H", "-p", "-o", "property,value,source", "all", name]).await?;
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
    let args = vec!["list", "-t", "snapshot", "-H", "-p", "-o", "name,used,refer,creation"];
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
    let raw = cmd::run(ZFS, &arg_refs).await?;
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
    cmd::run(ZFS, &["snapshot", &full]).await?;
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

#[derive(Debug, Deserialize)]
pub struct SnapshotDestroyQuery {
    pub name: String,
    pub recursive: Option<bool>,
}

pub async fn snapshot_destroy(
    State(state): State<AppState>,
    Query(q): Query<SnapshotDestroyQuery>,
) -> ApiResult<StatusCode> {
    let full = &q.name;
    validate_name(full)?;
    if !full.contains('@') {
        return Err(ApiError::BadRequest("not a snapshot name".into()));
    }
    let recursive = q.recursive.unwrap_or(false);
    if recursive {
        cmd::run(ZFS, &["destroy", "-R", full]).await?;
    } else {
        cmd::run(ZFS, &["destroy", full]).await?;
    }
    crate::audit::record(
        &state, None, "DELETE", "/api/zfs/snapshot/destroy", 200,
        Some(format!("destroyed snapshot {full}{}", if recursive { " (recursive)" } else { "" })),
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
    cmd::run(ZFS, &["rollback", "-r", full]).await?;
    crate::audit::record(
        &state, None, "POST", "/api/zfs/snapshot/rollback", 200,
        Some(format!("rolled back to {full}")),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct SnapshotCloneBody {
    pub source: String,
    pub target: String,
    pub mountpoint: Option<String>,
}

pub async fn snapshot_clone(
    State(state): State<AppState>,
    body: axum::Json<SnapshotCloneBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let source = body.source.trim();
    let target = body.target.trim();
    let mountpoint = body.mountpoint.as_deref().map(str::trim).filter(|s| !s.is_empty());
    validate_name(source)?;
    validate_name(target)?;
    if !source.contains('@') {
        return Err(ApiError::BadRequest("source must be a snapshot (contain @)".into()));
    }
    if target.contains('@') {
        return Err(ApiError::BadRequest("target must be a dataset name".into()));
    }
    if let Some(mp) = mountpoint {
        validate_mountpoint(mp)?;
        cmd::run(ZFS, &["clone", "-o", &format!("mountpoint={mp}"), source, target]).await?;
    } else {
        cmd::run(ZFS, &["clone", source, target]).await?;
    }
    crate::audit::record(
        &state, None, "POST", "/api/zfs/snapshot/clone", 201,
        Some(format!("cloned {source} → {target}")),
    );
    Ok((StatusCode::CREATED, Json(serde_json::json!({"name": target}))))
}
pub async fn pool_scrub(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    cmd::run(ZPOOL, &["scrub", &name]).await?;
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
    cmd::run(ZPOOL, &["scrub", "-s", &name]).await?;
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

/// Validate a disk device name. FreeBSD disk devices match ^[a-zA-Z0-9]+$
/// (ada0, da0, vtbd1, nvd0, mmcsd0s1, etc).  No path separators allowed;
/// caller may pass /dev/ prefix which we strip before validation.
fn validate_disk_name(name: &str) -> ApiResult<String> {
    let dev = name.strip_prefix("/dev/").unwrap_or(name);
    if dev.is_empty() || dev.len() > 32 {
        return Err(ApiError::BadRequest("invalid disk name length".into()));
    }
    let re = Regex::new(r"^[a-zA-Z0-9]+$").unwrap();
    if !re.is_match(dev) {
        return Err(ApiError::BadRequest("invalid disk name".into()));
    }
    Ok(dev.to_string())
}

/// Validate a vdev type keyword.
fn validate_vdev_type(t: &str) -> ApiResult<()> {
    match t {
        "disk" | "mirror" | "raidz1" | "raidz2" | "raidz3" => Ok(()),
        _ => Err(ApiError::BadRequest("invalid vdev type".into())),
    }
}

// ===== Pool Management: create / destroy / add / attach / detach / replace =====

/// A disk available for pool operations.
#[derive(Debug, Serialize)]
pub struct AvailableDisk {
    pub name: String,
    pub descr: String,
    pub size_bytes: u64,
    pub in_use: bool,
    pub pool: Option<String>,
}

pub async fn available_disks() -> ApiResult<Json<Vec<AvailableDisk>>> {
    // Get all physical disks via geom.
    let geom_raw = cmd::run("/sbin/geom", &["disk", "list"]).await.unwrap_or_default();

    let mut disks: Vec<(String, String, u64)> = Vec::new();
    let mut name = String::new();
    let mut descr = String::new();
    let mut size: u64 = 0;

    for line in geom_raw.lines() {
        let t = line.trim();
        let t = if t.starts_with(|c: char| c.is_ascii_digit()) {
            t.split_once(". ").map(|(_, rest)| rest.trim()).unwrap_or(t)
        } else {
            t
        };
        if let Some(v) = t.strip_prefix("Name:") {
            if size > 0 && !name.is_empty() {
                disks.push((name.clone(), descr.clone(), size));
            }
            name = v.trim().to_string();
            descr.clear();
            size = 0;
        } else if let Some(v) = t.strip_prefix("Mediasize:") {
            size = v.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
        } else if let Some(v) = t.strip_prefix("descr:") {
            descr = v.trim().to_string();
        }
    }
    if size > 0 && !name.is_empty() {
        disks.push((name, descr, size));
    }

    // Detect disks in active use. Two sources:
    //   1. `zpool status` — disks already part of a ZFS pool
    //   2. `mount` + `swapctl -l` — disks with mounted filesystems or swap
    //
    // A partition table alone (geom part list) is NOT a reason to exclude —
    // the user may intend to overwrite it.
    let disk_name_set: std::collections::HashSet<&str> =
        disks.iter().map(|(n, _, _)| n.as_str()).collect();
    let mut used_disks: HashMap<String, String> = HashMap::new();

    // Helper: map a device name (partition or whole disk) to a base disk in our list.
    fn match_base_disk<'a>(
        candidate: &str,
        disk_name_set: &'a std::collections::HashSet<&str>,
    ) -> Option<&'a str> {
        disk_name_set.iter().copied().find(|dn| {
            candidate == *dn
                || (candidate.starts_with(dn)
                    && candidate
                        .get(dn.len()..)
                        .map(|rest| rest.starts_with('p') || rest.starts_with('s'))
                        .unwrap_or(false))
        })
    }

    // 1) ZFS pool membership.
    let status_raw = cmd::run(ZPOOL, &["status"]).await.unwrap_or_default();
    let mut current_pool: Option<String> = None;
    for line in status_raw.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("pool:") {
            current_pool = Some(v.trim().to_string());
            continue;
        }
        let cols: Vec<&str> = t.split_whitespace().collect();
        if cols.is_empty() || current_pool.is_none() {
            continue;
        }
        let candidate = cols[0].strip_prefix("/dev/").unwrap_or(cols[0]);
        if let Some(dn) = match_base_disk(candidate, &disk_name_set) {
            used_disks
                .entry(dn.to_string())
                .or_insert_with(|| current_pool.clone().unwrap());
        }
    }

    // 2) Mounted filesystems: `mount` output lines start with "/dev/xxx on /path".
    let mount_raw = cmd::run("/sbin/mount", &["-p"]).await.unwrap_or_default();
    for line in mount_raw.lines() {
        let dev = line.split_whitespace().next().unwrap_or("");
        if !dev.starts_with("/dev/") {
            continue;
        }
        let candidate = dev.strip_prefix("/dev/").unwrap_or(dev);
        if let Some(dn) = match_base_disk(candidate, &disk_name_set) {
            used_disks
                .entry(dn.to_string())
                .or_insert_with(|| "mounted".to_string());
        }
    }

    // 3) Swap devices.
    let swap_raw = cmd::run("/sbin/swapctl", &["-l"]).await.unwrap_or_default();
    for line in swap_raw.lines() {
        let dev = line.split_whitespace().next().unwrap_or("");
        if !dev.starts_with("/dev/") {
            continue;
        }
        let candidate = dev.strip_prefix("/dev/").unwrap_or(dev);
        if let Some(dn) = match_base_disk(candidate, &disk_name_set) {
            used_disks
                .entry(dn.to_string())
                .or_insert_with(|| "swap".to_string());
        }
    }

    let result: Vec<AvailableDisk> = disks
        .into_iter()
        .map(|(name, descr, size_bytes)| {
            let (in_use, pool) = if let Some(p) = used_disks.get(&name) {
                (true, Some(p.clone()))
            } else {
                (false, None)
            };
            AvailableDisk {
                name,
                descr,
                size_bytes,
                in_use,
                pool,
            }
        })
        .collect();

    Ok(Json(result))
}

// ===== Pool import / export =====

/// A pool available for import (found on devices but not currently active).
#[derive(Debug, Default, Serialize)]
pub struct ImportablePool {
    pub name: String,
    pub id: String,
    pub state: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct ImportableQuery {
    pub include_destroyed: Option<bool>,
}

pub async fn pool_importable(
    Query(q): Query<ImportableQuery>,
) -> ApiResult<Json<Vec<ImportablePool>>> {
    let mut args = vec!["import"];
    if q.include_destroyed.unwrap_or(false) {
        args.push("-D");
    }
    let raw = cmd::run(ZPOOL, &args).await?;
    let mut pools: Vec<ImportablePool> = Vec::new();
    let mut current = ImportablePool {
        name: String::new(),
        id: String::new(),
        state: String::new(),
        size: 0,
    };
    let mut have = false;

    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("pool:") {
            if have && !current.name.is_empty() {
                pools.push(std::mem::take(&mut current));
            }
            current.name = rest.trim().to_string();
            have = true;
        } else if let Some(rest) = t.strip_prefix("id:") {
            current.id = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("state:") {
            current.state = rest.trim().to_string();
        } else if t.starts_with("config:") {
            // The config block follows; first line inside config is the
            // column header, second line is the pool with size. We scan
            // for a line whose first column is the pool name.
        } else {
            // Look for the pool summary line within config:
            // "  tank  4.00G  ONLINE  ..."
            let cols: Vec<&str> = t.split_whitespace().collect();
            if cols.len() >= 2 && have && cols[0] == current.name {
                current.size = cols
                    .iter()
                    .find_map(|c| {
                        let n = c.trim_end_matches(|ch: char| ch.is_alphabetic());
                        if n.is_empty() {
                            return None;
                        }
                        // Parse sizes like "4.00G", "500M", "1.00T"
                        if let Some(num) = n.parse::<f64>().ok() {
                            let suffix = c.trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
                            let mult = match suffix {
                                "K" | "KB" => 1024.0,
                                "M" | "MB" => 1024.0 * 1024.0,
                                "G" | "GB" => 1024.0 * 1024.0 * 1024.0,
                                "T" | "TB" => 1024.0_f64.powi(4),
                                "P" | "PB" => 1024.0_f64.powi(5),
                                _ => return None,
                            };
                            return Some((num * mult) as u64);
                        }
                        None
                    })
                    .unwrap_or(0);
            }
        }
    }
    if have && !current.name.is_empty() {
        pools.push(current);
    }

    Ok(Json(pools))
}

#[derive(Debug, Deserialize)]
pub struct PoolImportBody {
    pub name: String,
    pub force: Option<bool>,
    pub altroot: Option<String>,
    pub destroyed: Option<bool>,
}

pub async fn pool_import(
    State(state): State<AppState>,
    body: axum::Json<PoolImportBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let pool_name = body.name.trim();
    if pool_name.is_empty() || pool_name.contains('/') || pool_name.contains('@') {
        return Err(ApiError::BadRequest("invalid pool name".into()));
    }
    if !Regex::new(r"^[a-zA-Z0-9_.\-]+$").unwrap().is_match(pool_name) {
        return Err(ApiError::BadRequest("invalid pool name".into()));
    }

    let mut args: Vec<String> = vec!["import".into()];
    if body.destroyed.unwrap_or(false) {
        args.push("-D".into());
    }
    if body.force.unwrap_or(false) {
        args.push("-f".into());
    }
    if let Some(altroot) = &body.altroot {
        let altroot = altroot.trim();
        if !altroot.is_empty() {
            validate_mountpoint(altroot)?;
            args.push("-R".into());
            args.push(altroot.to_string());
        }
    }
    args.push(pool_name.to_string());

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cmd::run(ZPOOL, &arg_refs).await?;

    let mut detail = format!("imported pool {}", pool_name);
    if let Some(altroot) = &body.altroot {
        if !altroot.trim().is_empty() {
            detail = format!("{} (altroot: {})", detail, altroot);
        }
    }
    crate::audit::record(
        &state, None, "POST", "/api/zfs/pools/import", 200,
        Some(detail),
    );
    Ok((StatusCode::OK, Json(serde_json::json!({"name": pool_name}))))
}

pub async fn pool_export(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    cmd::run(ZPOOL, &["export", &name]).await?;
    crate::audit::record(
        &state, None, "POST", &format!("/api/zfs/pools/{}/export", name), 200,
        Some(format!("exported pool {}", name)),
    );
    Ok(StatusCode::OK)
}

/// One vdev group in a create/add request.
#[derive(Debug, Deserialize)]
pub struct VdevSpec {
    pub vdev_type: String,
    pub disks: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PoolCreateBody {
    pub name: String,
    pub ashift: Option<u32>,
    pub mountpoint: Option<String>,
    pub vdevs: Vec<VdevSpec>,
    pub properties: Option<HashMap<String, String>>,
}

/// Build zpool command args from a list of vdev specs.
/// Returns e.g. ["mirror", "vtbd1", "vtbd2", "raidz1", "vtbd3", "vtbd4", "vtbd5"]
fn build_vdev_args(vdevs: &[VdevSpec]) -> ApiResult<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    for vd in vdevs {
        if vd.disks.is_empty() {
            return Err(ApiError::BadRequest("vdev has no disks".into()));
        }
        validate_vdev_type(&vd.vdev_type)?;
        // type-specific minimum disk count
        let min = match vd.vdev_type.as_str() {
            "mirror" => 2,
            "raidz1" => 3,
            "raidz2" => 4,
            "raidz3" => 5,
            _ => 1, // plain disk
        };
        if vd.disks.len() < min {
            return Err(ApiError::BadRequest(format!(
                "{} requires at least {} disks",
                vd.vdev_type, min
            )));
        }
        if vd.vdev_type != "disk" {
            args.push(vd.vdev_type.clone());
        }
        for d in &vd.disks {
            args.push(validate_disk_name(d)?);
        }
    }
    Ok(args)
}

pub async fn pool_create(
    State(state): State<AppState>,
    body: axum::Json<PoolCreateBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let pool_name = body.name.trim();
    if pool_name.is_empty() || pool_name.contains('/') || pool_name.contains('@') {
        return Err(ApiError::BadRequest("invalid pool name".into()));
    }
    if !Regex::new(r"^[a-zA-Z][a-zA-Z0-9_.\-]*$").unwrap().is_match(pool_name) {
        return Err(ApiError::BadRequest("invalid pool name".into()));
    }
    if body.vdevs.is_empty() {
        return Err(ApiError::BadRequest("at least one vdev required".into()));
    }

    let vdev_args = build_vdev_args(&body.vdevs)?;

    let mut args: Vec<String> = vec!["create".into(), "-f".into()];

    // ashift is the most important property for pool creation.
    if let Some(ashift) = body.ashift {
        if ashift < 9 || ashift > 16 {
            return Err(ApiError::BadRequest("ashift must be 9-16".into()));
        }
        args.push("-o".into());
        args.push(format!("ashift={ashift}"));
    }

    if let Some(props) = &body.properties {
        for (k, v) in props {
            validate_prop_key(k)?;
            args.push("-o".into());
            args.push(format!("{k}={v}"));
        }
    }

    // mountpoint is a dataset-level property, passed via -O to zpool create.
    if let Some(mp) = &body.mountpoint {
        let mp = mp.trim();
        if !mp.is_empty() {
            validate_mountpoint(mp)?;
            args.push("-O".into());
            args.push(format!("mountpoint={mp}"));
        }
    }

    args.push(pool_name.to_string());
    args.extend(vdev_args);

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cmd::run(ZPOOL, &arg_refs).await?;

    let vdev_summary: Vec<String> = body
        .vdevs
        .iter()
        .map(|v| {
            if v.vdev_type == "disk" {
                v.disks.join(",")
            } else {
                format!("{}({})", v.vdev_type, v.disks.join(","))
            }
        })
        .collect();

    crate::audit::record(
        &state, None, "POST", "/api/zfs/pools", 201,
        Some(format!("created pool {} with {}", pool_name, vdev_summary.join("; "))),
    );
    Ok((StatusCode::CREATED, Json(serde_json::json!({"name": pool_name}))))
}

#[derive(Debug, Deserialize)]
pub struct PoolDestroyQuery {
    pub force: Option<bool>,
}

pub async fn pool_destroy(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<PoolDestroyQuery>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    let mut args = vec!["destroy"];
    if q.force.unwrap_or(false) {
        args.push("-f");
    }
    args.push(&name);
    cmd::run(ZPOOL, &args).await?;
    crate::audit::record(
        &state, None, "DELETE", &format!("/api/zfs/pools/{}", name), 200,
        Some(format!("destroyed pool {}", name)),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct PoolAddBody {
    pub vdevs: Vec<VdevSpec>,
    pub force: Option<bool>,
}

pub async fn pool_add_vdev(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: axum::Json<PoolAddBody>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    if body.vdevs.is_empty() {
        return Err(ApiError::BadRequest("at least one vdev required".into()));
    }
    let vdev_args = build_vdev_args(&body.vdevs)?;

    let mut args: Vec<String> = vec!["add".into()];
    if body.force.unwrap_or(false) {
        args.push("-f".into());
    }
    args.push(name.clone());
    args.extend(vdev_args);

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cmd::run(ZPOOL, &arg_refs).await?;

    let vdev_summary: Vec<String> = body
        .vdevs
        .iter()
        .map(|v| {
            if v.vdev_type == "disk" {
                v.disks.join(",")
            } else {
                format!("{}({})", v.vdev_type, v.disks.join(","))
            }
        })
        .collect();

    crate::audit::record(
        &state, None, "POST", &format!("/api/zfs/pools/{}/add", name), 200,
        Some(format!("added vdevs to {}: {}", name, vdev_summary.join("; "))),
    );
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub struct AttachBody {
    pub device: String,
    pub new_device: String,
}

pub async fn pool_attach(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: axum::Json<AttachBody>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    // device can be a disk name (vtbd1) or a vdev name (raidz1-0, mirror-0).
    let re = Regex::new(r"^[a-zA-Z0-9\-]+$").unwrap();
    if body.device.is_empty() || !re.is_match(&body.device) {
        return Err(ApiError::BadRequest("invalid device name".into()));
    }
    let device = &body.device;
    let new_device = validate_disk_name(&body.new_device)?;
    cmd::run(ZPOOL, &["attach", &name, device, &new_device]).await?;
    crate::audit::record(
        &state, None, "POST", &format!("/api/zfs/pools/{}/attach", name), 200,
        Some(format!("attached {} to {} in {}", new_device, device, name)),
    );
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub struct DeviceQuery {
    pub device: String,
}

pub async fn pool_detach(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<DeviceQuery>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    let device = validate_disk_name(&q.device)?;
    cmd::run(ZPOOL, &["detach", &name, &device]).await?;
    crate::audit::record(
        &state, None, "POST", &format!("/api/zfs/pools/{}/detach", name), 200,
        Some(format!("detached {} from {}", device, name)),
    );
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub struct ReplaceBody {
    pub old_device: String,
    pub new_device: String,
}

pub async fn pool_replace(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: axum::Json<ReplaceBody>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;
    let old_device = validate_disk_name(&body.old_device)?;
    let new_device = validate_disk_name(&body.new_device)?;
    cmd::run(ZPOOL, &["replace", &name, &old_device, &new_device]).await?;
    crate::audit::record(
        &state, None, "POST", &format!("/api/zfs/pools/{}/replace", name), 200,
        Some(format!("replaced {} with {} in {}", old_device, new_device, name)),
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
