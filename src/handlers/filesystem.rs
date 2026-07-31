//! Filesystem overview — disks, mounts, ZFS pools.

use std::collections::HashMap;

use axum::extract::Path;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::cmd;
use crate::error::{ApiError, ApiResult};
/// Path to smartctl (from the smartmontools port). Used for SMART health data
/// (power-on hours, temperature, attributes). Absent on a base system — the
/// handler degrades gracefully and reports the device as unsupported.
const SMARTCTL: &str = "/usr/local/sbin/smartctl";

#[derive(Debug, Serialize)]
pub struct FsOverview {
    pub disks: Vec<Disk>,
    pub mounts: Vec<Mount>,
    pub zpools: Vec<ZpoolSummary>,
}

#[derive(Debug, Serialize)]
pub struct Disk {
    pub name: String,
    pub descr: String,
    pub size_bytes: u64,
    pub rotation_rate: String,
}

#[derive(Debug, Serialize)]
pub struct Mount {
    pub device: String,
    pub mountpoint: String,
    pub fstype: String,
    pub size: u64,
    pub used: u64,
    pub available: u64,
    pub capacity_pct: f32,
    pub options: String,
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

pub async fn overview() -> ApiResult<Json<FsOverview>> {
    let result = tokio::task::spawn_blocking(|| FsOverview {
        disks: list_disks(),
        mounts: list_mounts(),
        zpools: list_zpools(),
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(result))
}

/// Detailed disk information — physical disk + partition table.
#[derive(Debug, Serialize)]
pub struct DiskDetail {
    pub name: String,
    pub descr: String,
    pub size_bytes: u64,
    pub sectorsize: u64,
    pub mode: String,
    pub ident: String,
    pub lunid: String,
    pub rotation_rate: String,
    pub fwsectors: u64,
    pub fwheads: u64,
    /// Partition scheme from geom part (e.g. "GPT", "MBR"); None if no table.
    pub scheme: Option<String>,
    pub state: Option<String>,
    pub first: Option<u64>,
    pub last: Option<u64>,
    pub entries: Option<u64>,
    pub partitions: Vec<Partition>,
}

#[derive(Debug, Serialize)]
pub struct Partition {
    pub name: String,
    pub mediasize_bytes: u64,
    pub sectorsize: u64,
    #[serde(rename = "type")]
    pub ptype: String,
    pub label: String,
    pub index: u32,
    pub start: u64,
    pub end: u64,
    pub offset_bytes: u64,
    pub rawuuid: String,
}

pub async fn disk_detail() -> ApiResult<Json<Vec<DiskDetail>>> {
    let result = tokio::task::spawn_blocking(list_disk_details)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(result))
}
// ── SMART health ──────────────────────────────────────────────────

/// SMART health snapshot for a single disk, exposed to the frontend.
///
/// Aggregates the most operationally relevant fields from `smartctl -j -a`:
/// overall health, power-on hours, temperature, power cycles, plus the full
/// ATA attribute table or the NVMe SMART/health log. `note` carries a reason
/// when the data is partial or unsupported (e.g. device open failed, SMART
/// disabled).
#[derive(Debug, Serialize)]
pub struct DiskSmart {
    /// Disk device name (e.g. "ada0").
    pub name: String,
    /// smartctl-detected device type (e.g. "ata", "sat", "atacam", "nvme").
    pub device_type: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// Overall SMART health: PASSED → true, FAILED → false. None when SMART
    /// is not supported or disabled on the device.
    pub healthy: Option<bool>,
    pub power_on_hours: Option<u64>,
    pub power_cycle_count: Option<u64>,
    /// Current temperature in °C.
    pub temperature: Option<i32>,
    /// ATA SMART attribute table (SATA/SAS disks); empty for NVMe.
    pub attributes: Vec<SmartAttr>,
    /// NVMe SMART/health log fields; None for ATA disks.
    pub nvme: Option<NvmeHealth>,
    pub note: Option<String>,
    /// True when the `smartctl` binary is not installed on the host. The
    /// frontend offers to install `smartmontools` via pkg in this case.
    #[serde(default)]
    pub smartctl_missing: bool,
}

#[derive(Debug, Serialize)]
pub struct SmartAttr {
    pub id: u32,
    pub name: String,
    /// Normalized value (vendor-specific, higher is better; typically 1–253).
    pub value: Option<u64>,
    pub worst: Option<u64>,
    pub thresh: Option<u64>,
    /// Raw counter value.
    pub raw: Option<u64>,
    pub raw_string: Option<String>,
    /// Normalized value at or below threshold (a failing/prefail attribute).
    pub failing: bool,
}

#[derive(Debug, Serialize)]
pub struct NvmeHealth {
    /// Estimated percentage of NVM endurance used (0–100+).
    pub percentage_used: Option<f64>,
    pub available_spare: Option<f64>,
    pub available_spare_threshold: Option<f64>,
    pub media_errors: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub controller_busy_time: Option<u64>,
}

/// `GET /api/filesystem/disks/{name}/smart` — SMART health for one disk.
///
/// Runs `smartctl -j -a /dev/<name>`. smartctl's exit code is a bitmask, not a
/// simple success/failure: bit 3 (`& 8`) means the disk is *failing* SMART
/// (still useful output), bit 1 (`& 2`) means the device could not be opened.
/// We therefore never treat a non-zero exit as an error — we parse stdout and
/// only surface a `note` when the data is absent.
pub async fn disk_smart(Path(name): Path<String>) -> ApiResult<Json<DiskSmart>> {
    validate_dev_name(&name)?;
    let dev = format!("/dev/{name}");
    let name = name.clone();
    // smartmontools not installed — return a structured marker so the frontend
    // can offer to install it, instead of an opaque 500.
    if !std::path::Path::new(SMARTCTL).exists() {
        return Ok(Json(empty_smart_missing(name)));
    }
    let output = cmd::run_output(SMARTCTL, &["-j", "-a", &dev]).await?;
    let code = output.status.code().unwrap_or(0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let root: SmartctlRoot = match serde_json::from_str(&stdout) {
        Ok(r) => r,
        Err(_) => {
            // No usable JSON — device unsupported, absent, or smartctl missing.
            let note = if code & 2 != 0 {
                stderr.trim().to_string()
            } else if stdout.trim().is_empty() {
                "smartctl produced no output".to_string()
            } else {
                "could not parse smartctl output".to_string()
            };
            return Ok(Json(empty_smart(name, note)));
        }
    };

    // Power-on hours / cycles: prefer smartctl's top-level提炼, fall back to
    // ATA attribute id 9/12, then the NVMe log.
    let mut power_on_hours = root.power_on_time.as_ref().and_then(|p| p.hours);
    let mut power_cycle_count = root.power_cycle_count;

    let mut attributes = Vec::new();
    if let Some(attrs) = &root.ata_smart_attributes {
        for a in &attrs.table {
            let failing = match (a.value, a.thresh) {
                (Some(v), Some(th)) if th > 0 => v <= th,
                _ => false,
            };
            let raw_val = a.raw.as_ref().and_then(|r| r.value);
            if a.id == 9 && power_on_hours.is_none() {
                power_on_hours = raw_val;
            }
            if a.id == 12 && power_cycle_count.is_none() {
                power_cycle_count = raw_val;
            }
            attributes.push(SmartAttr {
                id: a.id,
                name: a.name.clone(),
                value: a.value,
                worst: a.worst,
                thresh: a.thresh,
                raw: raw_val,
                raw_string: a.raw.as_ref().and_then(|r| r.string.clone()),
                failing,
            });
        }
    }

    let mut nvme = None;
    if let Some(log) = &root.nvme_smart_health_information_log {
        if power_on_hours.is_none() {
            power_on_hours = log.power_on_time.as_ref().and_then(|p| p.hours);
        }
        if power_cycle_count.is_none() {
            power_cycle_count = log.power_cycles;
        }
        nvme = Some(NvmeHealth {
            percentage_used: log.percentage_used,
            available_spare: log.available_spare,
            available_spare_threshold: log.available_spare_threshold,
            media_errors: log.media_errors,
            unsafe_shutdowns: log.unsafe_shutdowns,
            controller_busy_time: log.controller_busy_time,
        });
    }

    let temperature = root
        .temperature
        .as_ref()
        .and_then(|t| t.current)
        .or_else(|| {
            root.nvme_smart_health_information_log
                .as_ref()
                .and_then(|l| l.temperature)
        })
        .map(|v| v as i32);

    let mut result = DiskSmart {
        name,
        device_type: root.device.as_ref().and_then(|d| d.typ.clone()),
        model: root.model_name,
        serial: root.serial_number,
        healthy: root.smart_status.as_ref().and_then(|s| s.passed),
        power_on_hours,
        power_cycle_count,
        temperature,
        attributes,
        nvme,
        note: None,
        smartctl_missing: false,
    };
    // smartctl may emit a valid JSON skeleton without any SMART content (e.g.
    // an unsupported or absent device that still parses). Flag that explicitly
    // instead of returning an empty-looking record.
    if result.healthy.is_none()
        && result.attributes.is_empty()
        && result.nvme.is_none()
        && result.power_on_hours.is_none()
        && result.power_cycle_count.is_none()
        && result.temperature.is_none()
    {
        result.note = Some("no SMART data reported".to_string());
    }
    Ok(Json(result))
}

fn empty_smart(name: String, note: String) -> DiskSmart {
    DiskSmart {
        name,
        device_type: None,
        model: None,
        serial: None,
        healthy: None,
        power_on_hours: None,
        power_cycle_count: None,
        temperature: None,
        attributes: vec![],
        nvme: None,
        note: Some(note),
        smartctl_missing: false,
    }
}

/// Marker record returned when `smartctl` is not installed.
fn empty_smart_missing(name: String) -> DiskSmart {
    DiskSmart {
        name,
        device_type: None,
        model: None,
        serial: None,
        healthy: None,
        power_on_hours: None,
        power_cycle_count: None,
        temperature: None,
        attributes: vec![],
        nvme: None,
        note: None,
        smartctl_missing: true,
    }
}

/// Validate a GEOM disk device name (e.g. `ada0`, `da0`, `nda0`).
/// Rejects path separators and other shell-meta characters defensively.
fn validate_dev_name(name: &str) -> ApiResult<()> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(ApiError::BadRequest("invalid device name".into()));
    }
    Ok(())
}

// ── smartctl JSON schema (subset; all fields optional — varies by protocol) ──

#[derive(Deserialize)]
struct SmartctlRoot {
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    serial_number: Option<String>,
    #[serde(default)]
    device: Option<SmartctlDevice>,
    #[serde(default)]
    smart_status: Option<SmartctlSmartStatus>,
    #[serde(default)]
    temperature: Option<SmartctlTemp>,
    #[serde(default)]
    power_on_time: Option<SmartctlPowerOn>,
    #[serde(default)]
    power_cycle_count: Option<u64>,
    #[serde(default)]
    ata_smart_attributes: Option<SmartctlAtaAttrs>,
    #[serde(default)]
    nvme_smart_health_information_log: Option<SmartctlNvmeLog>,
}

#[derive(Deserialize)]
struct SmartctlDevice {
    #[serde(default, rename = "type")]
    typ: Option<String>,
}

#[derive(Deserialize)]
struct SmartctlSmartStatus {
    #[serde(default)]
    passed: Option<bool>,
}

#[derive(Deserialize)]
struct SmartctlTemp {
    #[serde(default)]
    current: Option<i64>,
}

#[derive(Deserialize)]
struct SmartctlPowerOn {
    #[serde(default)]
    hours: Option<u64>,
}

#[derive(Deserialize)]
struct SmartctlAtaAttrs {
    #[serde(default)]
    table: Vec<SmartctlAttr>,
}

#[derive(Deserialize)]
struct SmartctlAttr {
    #[serde(default)]
    id: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: Option<u64>,
    #[serde(default)]
    worst: Option<u64>,
    #[serde(default)]
    thresh: Option<u64>,
    #[serde(default)]
    raw: Option<SmartctlRaw>,
}

#[derive(Deserialize)]
struct SmartctlRaw {
    #[serde(default)]
    value: Option<u64>,
    #[serde(default)]
    string: Option<String>,
}

#[derive(Deserialize)]
struct SmartctlNvmeLog {
    #[serde(default)]
    percentage_used: Option<f64>,
    #[serde(default)]
    available_spare: Option<f64>,
    #[serde(default)]
    available_spare_threshold: Option<f64>,
    #[serde(default)]
    media_errors: Option<u64>,
    #[serde(default)]
    unsafe_shutdowns: Option<u64>,
    #[serde(default)]
    controller_busy_time: Option<u64>,
    #[serde(default)]
    power_cycles: Option<u64>,
    #[serde(default)]
    power_on_time: Option<SmartctlPowerOn>,
    #[serde(default)]
    temperature: Option<i64>,
}


/// Parse `geom disk list` for physical disks. Skips zero-size devices (cd0).
fn list_disks() -> Vec<Disk> {
    let raw = cmd::run_sync("/sbin/geom", &["disk", "list"]).unwrap_or_default();

    let mut disks = Vec::new();
    let mut name = String::new();
    let mut descr = String::new();
    let mut size: u64 = 0;
    let mut rotation = String::new();

    for line in raw.lines() {
        let t = line.trim();
        // Lines like "1. Name: ada0" — strip leading "N. " prefix.
        let t = if t.starts_with(|c: char| c.is_ascii_digit()) {
            t.split_once(". ").map(|(_, rest)| rest.trim()).unwrap_or(t)
        } else {
            t
        };
        if let Some(v) = t.strip_prefix("Name:") {
            // Flush previous disk if non-empty.
            if size > 0 && !name.is_empty() {
                disks.push(Disk {
                    name: name.clone(),
                    descr: descr.clone(),
                    size_bytes: size,
                    rotation_rate: rotation.clone(),
                });
            }
            name = v.trim().to_string();
            descr.clear();
            size = 0;
            rotation.clear();
        } else if let Some(v) = t.strip_prefix("Mediasize:") {
            // "Mediasize: 2000398934016 (1.8T)"
            size = v.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
        } else if let Some(v) = t.strip_prefix("descr:") {
            descr = v.trim().to_string();
        } else if let Some(v) = t.strip_prefix("rotationrate:") {
            rotation = v.trim().to_string();
        }
    }
    // Flush last disk.
    if size > 0 && !name.is_empty() {
        disks.push(Disk {
            name,
            descr,
            size_bytes: size,
            rotation_rate: rotation,
        });
    }
    disks
}

/// Parse `geom disk list` + `geom part list` for detailed disk information.
/// Skips zero-size devices (cd0).
fn list_disk_details() -> Vec<DiskDetail> {
    // --- base disk fields from `geom disk list` ---
    let mut disks: std::collections::HashMap<String, DiskDetail> = HashMap::new();
    if let Ok(raw) = cmd::run_sync("/sbin/geom", &["disk", "list"]) {
            let raw = raw.as_str();
            let mut cur = DiskDetail {
                name: String::new(), descr: String::new(), size_bytes: 0,
                sectorsize: 0, mode: String::new(), ident: String::new(),
                lunid: String::new(), rotation_rate: String::new(),
                fwsectors: 0, fwheads: 0, scheme: None, state: None,
                first: None, last: None, entries: None, partitions: vec![],
            };
            let mut have = false;
            for line in raw.lines() {
                let t = line.trim();
                let t = if t.starts_with(|c: char| c.is_ascii_digit()) {
                    t.split_once(". ").map(|(_, r)| r.trim()).unwrap_or(t)
                } else {
                    t
                };
                if let Some(v) = t.strip_prefix("Name:") {
                    if have && cur.size_bytes > 0 {
                        disks.insert(cur.name.clone(), cur);
                    }
                    cur = DiskDetail {
                        name: v.trim().to_string(), descr: String::new(), size_bytes: 0,
                        sectorsize: 0, mode: String::new(), ident: String::new(),
                        lunid: String::new(), rotation_rate: String::new(),
                        fwsectors: 0, fwheads: 0, scheme: None, state: None,
                        first: None, last: None, entries: None, partitions: vec![],
                    };
                    have = true;
                } else if have {
                    if let Some(v) = t.strip_prefix("Mediasize:") {
                        cur.size_bytes = v.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
                    } else if let Some(v) = t.strip_prefix("Sectorsize:") {
                        cur.sectorsize = v.trim().parse().unwrap_or(0);
                    } else if let Some(v) = t.strip_prefix("Mode:") {
                        cur.mode = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("descr:") {
                        cur.descr = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("lunid:") {
                        cur.lunid = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("ident:") {
                        cur.ident = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("rotationrate:") {
                        cur.rotation_rate = v.trim().to_string();
                    } else if let Some(v) = t.strip_prefix("fwsectors:") {
                        cur.fwsectors = v.trim().parse().unwrap_or(0);
                    } else if let Some(v) = t.strip_prefix("fwheads:") {
                        cur.fwheads = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            if have && cur.size_bytes > 0 {
                disks.insert(cur.name.clone(), cur);
            }
    }

    // --- partition table from `geom part list` ---
    if let Ok(raw) = cmd::run_sync("/sbin/geom", &["part", "list"]) {
        parse_geom_part(&raw, &mut disks);
    }

    // Preserve geom order: sort by name (ada0, ada1, da0, ...).
    let mut details: Vec<DiskDetail> = disks.into_values().collect();
    details.sort_by(|a, b| a.name.cmp(&b.name));
    details
}

/// Parse `geom part list` output and attach partition info to matching disks.
/// Each geom block: `Geom name: X`, top-level metadata, `Providers:` (partitions),
/// then `Consumers:`. Provider lines start with `N. Name: foo`.
fn parse_geom_part(raw: &str, disks: &mut HashMap<String, DiskDetail>) {
    // State machine per geom block.
    let mut cur_name: Option<String> = None;
    // Sections within a block: top-level metadata, "providers", "consumers".
    let mut in_providers = false;
    let mut cur_part = Partition {
        name: String::new(), mediasize_bytes: 0, sectorsize: 0,
        ptype: String::new(), label: String::new(), index: 0,
        start: 0, end: 0, offset_bytes: 0, rawuuid: String::new(),
    };
    let mut have_part = false;

    let flush_part = |have_part: &mut bool, cur_name: &Option<String>, cur_part: &mut Partition, disks: &mut HashMap<String, DiskDetail>| {
        if *have_part {
            if let Some(n) = cur_name {
                if let Some(d) = disks.get_mut(n) {
                    d.partitions.push(Partition {
                        name: cur_part.name.clone(),
                        mediasize_bytes: cur_part.mediasize_bytes,
                        sectorsize: cur_part.sectorsize,
                        ptype: cur_part.ptype.clone(),
                        label: cur_part.label.clone(),
                        index: cur_part.index,
                        start: cur_part.start,
                        end: cur_part.end,
                        offset_bytes: cur_part.offset_bytes,
                        rawuuid: cur_part.rawuuid.clone(),
                    });
                }
            }
            *cur_part = Partition {
                name: String::new(), mediasize_bytes: 0, sectorsize: 0,
                ptype: String::new(), label: String::new(), index: 0,
                start: 0, end: 0, offset_bytes: 0, rawuuid: String::new(),
            };
            *have_part = false;
        }
    };

    for line in raw.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("Geom name:") {
            // Flush last partition of previous block.
            flush_part(&mut have_part, &cur_name, &mut cur_part, disks);
            cur_name = Some(v.trim().to_string());
            in_providers = false;
            continue;
        }
        if t == "Providers:" {
            in_providers = true;
            continue;
        }
        if t == "Consumers:" {
            flush_part(&mut have_part, &cur_name, &mut cur_part, disks);
            in_providers = false;
            continue;
        }
        let Some(n) = cur_name.clone() else { continue };

        if in_providers {
            // Provider header: "N. Name: ada0p1".
            let stripped = t
                .strip_prefix(|c: char| c.is_ascii_digit())
                .and_then(|s| s.strip_prefix(". "))
                .map(|s| s.trim());
            if let Some(rest) = stripped {
                if let Some(v) = rest.strip_prefix("Name:") {
                    flush_part(&mut have_part, &cur_name, &mut cur_part, disks);
                    cur_part.name = v.trim().to_string();
                    have_part = true;
                    continue;
                }
            }
            if have_part {
                if let Some(v) = t.strip_prefix("Mediasize:") {
                    cur_part.mediasize_bytes = v.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
                } else if let Some(v) = t.strip_prefix("Sectorsize:") {
                    cur_part.sectorsize = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = t.strip_prefix("type:") {
                    cur_part.ptype = v.trim().to_string();
                } else if let Some(v) = t.strip_prefix("label:") {
                    cur_part.label = v.trim().to_string();
                } else if let Some(v) = t.strip_prefix("index:") {
                    cur_part.index = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = t.strip_prefix("start:") {
                    cur_part.start = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = t.strip_prefix("end:") {
                    cur_part.end = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = t.strip_prefix("offset:") {
                    cur_part.offset_bytes = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = t.strip_prefix("rawuuid:") {
                    cur_part.rawuuid = v.trim().to_string();
                }
            }
        } else {
            // Top-level geom metadata.
            if let Some(d) = disks.get_mut(&n) {
                if let Some(v) = t.strip_prefix("scheme:") {
                    d.scheme = Some(v.trim().to_string());
                } else if let Some(v) = t.strip_prefix("state:") {
                    d.state = Some(v.trim().to_string());
                } else if let Some(v) = t.strip_prefix("first:") {
                    d.first = v.trim().parse().ok();
                } else if let Some(v) = t.strip_prefix("last:") {
                    d.last = v.trim().parse().ok();
                } else if let Some(v) = t.strip_prefix("entries:") {
                    d.entries = v.trim().parse().ok();
                }
            }
        }
    }
    // Flush trailing partition of last block.
    flush_part(&mut have_part, &cur_name, &mut cur_part, disks);
}

/// Parse `mount` for mounted filesystems.
fn list_mounts() -> Vec<Mount> {
    let raw = cmd::run_sync("/sbin/mount", &[]).unwrap_or_default();
    let mut mounts = Vec::new();
    for line in raw.lines() {
        // Format: "device on /mountpoint (fstype, options)"
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() < 4 {
            continue;
        }
        if parts[1] != "on" {
            continue;
        }
        let device = parts[0].to_string();
        let mountpoint = parts[2].to_string();
        let rest = parts[3];
        // Extract "(fstype, options)" — fstype is first entry in parens.
        let paren = match rest.find('(') {
            Some(i) => &rest[i + 1..],
            None => continue,
        };
        let paren_end = paren.rfind(')').unwrap_or(paren.len());
        let inner = &paren[..paren_end];
        let fstype = inner.split(',').next().unwrap_or("").trim().to_string();
        let options = inner.to_string();
        mounts.push(Mount {
            device,
            mountpoint,
            fstype,
            size: 0,
            used: 0,
            available: 0,
            capacity_pct: 0.0,
            options,
        });
    }
    // Enrich with df data for size/used/avail.
    enrich_with_df(&mut mounts);
    mounts
}

/// Parse `df -k` (1K-blocks) and fill in size/used/available for matching mounts.
fn enrich_with_df(mounts: &mut [Mount]) {
    let raw = match cmd::run_sync("/bin/df", &["-k"]) {
        Ok(s) => s,
        _ => return,
    };
    // Build a map of mountpoint → (size, used, avail, capacity)
    let mut df_map: HashMap<String, (u64, u64, u64, f32)> = HashMap::new();
    for line in raw.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 {
            continue;
        }
        let mountpoint = cols[5].to_string();
        let size = cols[1].parse::<u64>().unwrap_or(0) * 1024;
        let used = cols[2].parse::<u64>().unwrap_or(0) * 1024;
        let avail = cols[3].parse::<u64>().unwrap_or(0) * 1024;
        let cap = cols[4].trim_end_matches('%').parse::<f32>().unwrap_or(0.0);
        df_map.insert(mountpoint, (size, used, avail, cap));
    }
    for m in mounts.iter_mut() {
        if let Some(&(size, used, avail, cap)) = df_map.get(&m.mountpoint) {
            m.size = size;
            m.used = used;
            m.available = avail;
            m.capacity_pct = cap;
        }
    }
}


/// Parse `zpool list -H -p` for ZFS pool summaries.
fn list_zpools() -> Vec<ZpoolSummary> {
    let raw = cmd::run_sync("/sbin/zpool", &["list", "-H", "-p"]).unwrap_or_default();
    let mut pools = Vec::new();
    // Columns: NAME SIZE ALLOC FREE CKPOINT EXPANDSZ FRAG CAP DEDUP HEALTH ALTROOT
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 {
            continue;
        }
        let parse = |i: usize| -> u64 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
        let parsef = |i: usize| -> f32 { cols.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
        pools.push(ZpoolSummary {
            name: cols[0].to_string(),
            size: parse(1),
            allocated: parse(2),
            free: parse(3),
            fragmentation_pct: parsef(6),
            capacity_pct: parsef(7),
            dedup: parsef(8),
            health: cols.get(9).unwrap_or(&"").to_string(),
        });
    }
    pools
}
