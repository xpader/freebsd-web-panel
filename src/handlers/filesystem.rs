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
    Ok(Json(list_disk_details()))
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

/// Parse `geom disk list` + `geom part list` for detailed disk information.
/// Skips zero-size devices (cd0).
fn list_disk_details() -> Vec<DiskDetail> {
    // --- base disk fields from `geom disk list` ---
    let mut disks: std::collections::HashMap<String, DiskDetail> = HashMap::new();
    let out = Command::new("/sbin/geom").args(["disk", "list"]).output();
    if let Ok(o) = out {
        if o.status.success() {
            let raw = String::from_utf8_lossy(&o.stdout);
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
    }

    // --- partition table from `geom part list` ---
    let out = Command::new("/sbin/geom").args(["part", "list"]).output();
    if let Ok(o) = out {
        if o.status.success() {
            let raw = String::from_utf8_lossy(&o.stdout);
            parse_geom_part(&raw, &mut disks);
        }
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
