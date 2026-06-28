//! System information & live metrics endpoints.

use std::sync::LazyLock;

use axum::Json;
use parking_lot::Mutex;
use serde::Serialize;

use crate::error::ApiResult;

// ---- Static system info ----

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub hostname: String,
    pub os_release: String,
    pub os_version: String,
    pub kernel: String,
    pub uptime_seconds: u64,
    pub boot_time: i64,
    pub loadavg: [f64; 3],
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub memory_total: u64,
    pub swap_total: u64,
}

pub async fn system_info() -> ApiResult<Json<SystemInfo>> {
    let hostname = read_sysctl("kern.hostname").unwrap_or_else(|_| "unknown".into());
    let os_release = read_sysctl("kern.osrelease").unwrap_or_default();
    let os_version = read_sysctl("kern.osreldate").unwrap_or_default();
    let kernel = read_sysctl("kern.ident").unwrap_or_default();
    let cpu_model = read_sysctl("hw.model").unwrap_or_else(|_| "unknown".into());
    let cpu_cores: u32 = read_sysctl("hw.ncpu")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);
    let memory_total: u64 = read_sysctl("hw.physmem")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let swap_total = read_swap_total();

    let boot_time = parse_boottime(&read_sysctl("kern.boottime").unwrap_or_default());
    let now = now_ts();
    let uptime = if boot_time > 0 {
        (now - boot_time).max(0) as u64
    } else {
        0
    };

    Ok(Json(SystemInfo {
        hostname,
        os_release,
        os_version,
        kernel,
        uptime_seconds: uptime,
        boot_time,
        loadavg: read_loadavg(),
        cpu_model,
        cpu_cores,
        memory_total,
        swap_total,
    }))
}

fn parse_boottime(raw: &str) -> i64 {
    // { sec = 1234, usec = 0 } Wed Jun ...
    let sec_eq = match raw.find("sec = ") {
        Some(i) => i,
        None => return 0,
    };
    let rest = &raw[sec_eq + 6..];
    let end = rest.find(',').unwrap_or(rest.len());
    rest[..end].trim().parse().unwrap_or(0)
}

// ---- Live metrics ----

#[derive(Debug, Serialize)]
pub struct SystemMetrics {
    pub timestamp: i64,
    pub uptime_seconds: u64,
    pub loadavg: [f64; 3],
    pub cpu_usage: f32,        // overall 0..100
    pub cpu_usage_per_core: Vec<f32>,
    pub cpu_freq_mhz: u32,
    pub memory: MemMetrics,
    pub swap: SwapMetrics,
    pub temperatures: Vec<TempReading>,
    pub processes: u64,
}

#[derive(Debug, Serialize)]
pub struct MemMetrics {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub cached: u64,
    pub wired: u64,
    pub usage: f32, // 0..100
}

#[derive(Debug, Serialize)]
pub struct SwapMetrics {
    pub total: u64,
    pub used: u64,
    pub usage: f32,
}

#[derive(Debug, Serialize)]
pub struct TempReading {
    pub source: String,
    pub value: f32, // Celsius
}

/// Previous CPU-times sample, used to compute usage delta.
/// Stored as flat vec (5 values per core: user,nice,sys,intr,idle).
static LAST_CP_TIMES: LazyLock<Mutex<Option<CpuSample>>> =
    LazyLock::new(|| Mutex::new(None));

struct CpuSample {
    times: Vec<u64>, // flat: 5 * ncpu
}

pub async fn system_metrics() -> ApiResult<Json<SystemMetrics>> {
    let now = now_ts();
    let page_size: u64 = read_sysctl("vm.stats.vm.v_page_size")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(4096);

    // Memory from vm.stats.vm.*
    let vpc = |name: &str| -> u64 {
        read_sysctl(name)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    let page_count = vpc("vm.stats.vm.v_page_count");
    let free_count = vpc("vm.stats.vm.v_free_count");
    let active = vpc("vm.stats.vm.v_active_count");
    let inactive = vpc("vm.stats.vm.v_inactive_count");
    let wire = vpc("vm.stats.vm.v_wire_count");
    let cache = vpc("vm.stats.vm.v_cache_count");
    let mem_total = page_size * page_count;
    let mem_used = page_size * (active + wire);
    let mem_free = page_size * (free_count + inactive + cache);
    let mem_usage = if mem_total > 0 {
        (mem_used as f32 / mem_total as f32) * 100.0
    } else {
        0.0
    };

    // Swap
    let (swap_total, swap_used) = read_swap();
    let swap_usage = if swap_total > 0 {
        (swap_used as f32 / swap_total as f32) * 100.0
    } else {
        0.0
    };

    // CPU usage from kern.cp_times (sample delta).
    let cp_times_raw = read_sysctl("kern.cp_times").unwrap_or_default();
    let times: Vec<u64> = cp_times_raw
        .split_whitespace()
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let ncpu = (times.len() / 5).max(1);
    let (cpu_usage, per_core) = {
        let mut guard = LAST_CP_TIMES.lock();
        let mut per: Vec<f32> = Vec::with_capacity(ncpu);
        let mut sum_busy = 0u64;
        let mut sum_total = 0u64;
        if let Some(prev) = guard.as_ref() {
            if prev.times.len() == times.len() {
                for i in 0..ncpu {
                    let off = i * 5;
                    let d_user = times[off].saturating_sub(prev.times[off]);
                    let d_nice = times[off + 1].saturating_sub(prev.times[off + 1]);
                    let d_sys = times[off + 2].saturating_sub(prev.times[off + 2]);
                    let d_intr = times[off + 3].saturating_sub(prev.times[off + 3]);
                    let d_idle = times[off + 4].saturating_sub(prev.times[off + 4]);
                    let total = d_user + d_nice + d_sys + d_intr + d_idle;
                    let busy = d_user + d_nice + d_sys + d_intr;
                    per.push(if total > 0 {
                        (busy as f32 / total as f32) * 100.0
                    } else {
                        0.0
                    });
                    sum_busy += busy;
                    sum_total += total;
                }
            }
        }
        *guard = Some(CpuSample {
            times: times.clone(),
        });
        let overall = if sum_total > 0 {
            (sum_busy as f32 / sum_total as f32) * 100.0
        } else {
            0.0
        };
        if per.is_empty() {
            per = vec![0.0; ncpu];
        }
        (overall, per)
    };

    let cpu_freq_mhz: u32 = read_sysctl("dev.cpu.0.freq")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    // Temperatures: dev.cpu.N.temperature
    let temperatures = read_all_temps();

    let processes: u64 = read_sysctl("kern.lastpid")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let boot_time = parse_boottime(&read_sysctl("kern.boottime").unwrap_or_default());
    let uptime = if boot_time > 0 {
        (now - boot_time).max(0) as u64
    } else {
        0
    };

    Ok(Json(SystemMetrics {
        timestamp: now,
        uptime_seconds: uptime,
        loadavg: read_loadavg(),
        cpu_usage,
        cpu_usage_per_core: per_core,
        cpu_freq_mhz,
        memory: MemMetrics {
            total: mem_total,
            used: mem_used,
            free: mem_free,
            cached: page_size * cache,
            wired: page_size * wire,
            usage: mem_usage,
        },
        swap: SwapMetrics {
            total: swap_total,
            used: swap_used,
            usage: swap_usage,
        },
        temperatures,
        processes,
    }))
}

/// Enumerate `sysctl -aN` to find dev.cpu.*.temperature entries, then read each.
fn read_all_temps() -> Vec<TempReading> {
    let names = sysctl_names_matching(|n| {
        n.starts_with("dev.cpu.") && n.ends_with(".temperature")
    });
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if let Ok(raw) = read_sysctl(&name) {
            // "44.0C"
            let num: String = raw.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Ok(v) = num.parse::<f32>() {
                let core = name
                    .strip_prefix("dev.cpu.")
                    .and_then(|s| s.split('.').next())
                    .unwrap_or("?");
                out.push(TempReading {
                    source: format!("CPU {core}"),
                    value: v,
                });
            }
        }
    }
    out
}

/// Run `sysctl -aN` once (cached) and filter names via predicate.
fn sysctl_names_matching<F: Fn(&str) -> bool>(pred: F) -> Vec<String> {
    static ALL_NAMES: LazyLock<Mutex<Option<Vec<String>>>> =
        LazyLock::new(|| Mutex::new(None));
    let names: Vec<String> = {
        let mut g = ALL_NAMES.lock();
        if g.is_none() {
            *g = Some(read_all_sysctl_names());
        }
        g.as_ref().unwrap().clone()
    };
    names.into_iter().filter(|n| pred(n)).collect()
}

fn read_all_sysctl_names() -> Vec<String> {
    let out = std::process::Command::new("/sbin/sysctl")
        .arg("-aN")
        .output();
    match out {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => Vec::new(),
    }
}

fn read_swap() -> (u64, u64) {
    // swapinfo -k (1K-blocks)
    let out = std::process::Command::new("/usr/sbin/swapinfo")
        .arg("-k")
        .output();
    let (mut total, mut used) = (0u64, 0u64);
    if let Ok(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 3 {
                total += cols[1].parse::<u64>().unwrap_or(0) * 1024;
                used += cols[2].parse::<u64>().unwrap_or(0) * 1024;
            }
        }
    }
    (total, used)
}

fn read_swap_total() -> u64 {
    read_swap().0
}

fn read_loadavg() -> [f64; 3] {
    let mut la = [0.0_f64; 3];
    if let Ok(out) = std::process::Command::new("/usr/bin/uptime").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        let nums: Vec<f64> = s
            .split_whitespace()
            .filter_map(|t| {
                t.trim_matches(|c: char| !c.is_ascii_digit() && c != '.')
                    .parse()
                    .ok()
            })
            .collect();
        if nums.len() >= 3 {
            let n = nums.len();
            la[0] = nums[n - 3];
            la[1] = nums[n - 2];
            la[2] = nums[n - 1];
        }
    }
    la
}

fn read_sysctl(name: &str) -> std::io::Result<String> {
    let out = std::process::Command::new("/sbin/sysctl")
        .arg("-n")
        .arg(name)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("sysctl {name} failed"),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn now_ts() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
