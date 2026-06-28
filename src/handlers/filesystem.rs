//! Filesystem overview — disks, mounts, ZFS pools.

use std::collections::HashMap;
use std::process::Command;
use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

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
    Ok(Json(FsOverview {
        disks: list_disks(),
        mounts: list_mounts(),
        zpools: list_zpools(),
    }))
}

/// Parse `geom disk list` for physical disks. Skips zero-size devices (cd0).
fn list_disks() -> Vec<Disk> {
    let out = Command::new("/sbin/geom")
        .args(["disk", "list"])
        .output();
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };

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

/// Parse `mount` for mounted filesystems.
fn list_mounts() -> Vec<Mount> {
    let out = Command::new("/sbin/mount").output();
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };
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
    let out = Command::new("/bin/df")
        .args(["-k"])
        .output();
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
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
    let out = Command::new("/sbin/zpool")
        .args(["list", "-H", "-p"])
        .output();
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };
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
