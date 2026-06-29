//! System information & live metrics endpoints.

use std::sync::LazyLock;

use axum::Json;
use parking_lot::Mutex;
use serde::Serialize;

use crate::error::ApiResult;
use crate::sysinfo;

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
    let hostname = sysinfo::read_string("kern.hostname").unwrap_or_else(|| "unknown".into());
    let os_release = sysinfo::read_string("kern.osrelease").unwrap_or_default();
    let os_version = sysinfo::read_string("kern.osreldate").unwrap_or_default();
    let kernel = sysinfo::read_string("kern.ident").unwrap_or_default();
    let cpu_model = sysinfo::read_string("hw.model").unwrap_or_else(|| "unknown".into());
    let cpu_cores: u32 = sysinfo::read_u64("hw.ncpu").unwrap_or(1) as u32;
    let memory_total: u64 = sysinfo::read_u64("hw.physmem").unwrap_or(0);
    let swap_total = read_swap_total();

    let boot_time = sysinfo::boot_time();
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
        loadavg: sysinfo::read_loadavg(),
        cpu_model,
        cpu_cores,
        memory_total,
        swap_total,
    }))
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
    let page_size: u64 = sysinfo::read_u64("vm.stats.vm.v_page_size").unwrap_or(4096);

    // Memory from vm.stats.vm.*
    let vpc = |name: &str| -> u64 { sysinfo::read_u64(name).unwrap_or(0) };
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
    let times: Vec<u64> = sysinfo::read_cp_times();
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

    let cpu_freq_mhz: u32 = sysinfo::read_u64("dev.cpu.0.freq").unwrap_or(0) as u32;

    // Temperatures: dev.cpu.N.temperature
    let temperatures = sysinfo::read_core_temps(ncpu as u32)
        .into_iter()
        .map(|(i, v)| TempReading {
            source: format!("CPU {i}"),
            value: v,
        })
        .collect();

    let processes: u64 = sysinfo::read_u64("kern.lastpid").unwrap_or(0);

    let boot_time = sysinfo::boot_time();
    let uptime = if boot_time > 0 {
        (now - boot_time).max(0) as u64
    } else {
        0
    };

    Ok(Json(SystemMetrics {
        timestamp: now,
        uptime_seconds: uptime,
        loadavg: sysinfo::read_loadavg(),
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

fn now_ts() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
