//! System information & live metrics endpoints.

use std::collections::HashMap;
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
    pub network: Vec<NetIface>,
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

#[derive(Debug, Serialize)]
pub struct NetIface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_rate: f64, // bytes/sec
    pub tx_rate: f64, // bytes/sec
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub up: bool,
    pub status: String,
    pub media: String,
    pub ipv4: Vec<String>,
    pub mac: Option<String>,
    pub mtu: u32,
}

/// Previous CPU-times sample, used to compute usage delta.
/// Stored as flat vec (5 values per core: user,nice,sys,intr,idle).
static LAST_CP_TIMES: LazyLock<Mutex<Option<CpuSample>>> =
    LazyLock::new(|| Mutex::new(None));

struct CpuSample {
    times: Vec<u64>, // flat: 5 * ncpu
}

/// Previous network counters + timestamp, used to compute live rate.
static LAST_NET: LazyLock<Mutex<Option<NetSample>>> =
    LazyLock::new(|| Mutex::new(None));

struct NetSample {
    ts: i64,
    counters: HashMap<String, sysinfo::NetCounters>,
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

    // Network interfaces — counters + metadata, with rate computed from delta.
    let network = collect_network(now);

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
        network,
    }))
}

/// Gather network interfaces: counters from `netstat`, metadata from `ifconfig`,
/// and rate (bytes/sec) computed against the previous poll's counters.
fn collect_network(now: i64) -> Vec<NetIface> {
    let counters = sysinfo::read_net_counters();
    let infos = sysinfo::read_net_info();

    let (rx_rate, tx_rate) = {
        let mut guard = LAST_NET.lock();
        let mut rx_map = HashMap::new();
        let mut tx_map = HashMap::new();
        if let Some(ref prev) = *guard {
            let dt = (now - prev.ts).max(1) as f64;
            for (name, cur) in &counters {
                if let Some(p) = prev.counters.get(name) {
                    rx_map.insert(
                        name.clone(),
                        cur.rx_bytes.saturating_sub(p.rx_bytes) as f64 / dt,
                    );
                    tx_map.insert(
                        name.clone(),
                        cur.tx_bytes.saturating_sub(p.tx_bytes) as f64 / dt,
                    );
                }
            }
        }
        *guard = Some(NetSample {
            ts: now,
            counters: counters.clone(),
        });
        (rx_map, tx_map)
    };

    // Merge counters with interface metadata. Interfaces present in counters
    // but missing from ifconfig (unlikely) still appear with empty metadata.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(infos.len());
    for info in &infos {
        seen.insert(info.name.clone());
        let c = counters.get(&info.name);
        out.push(NetIface {
            rx_bytes: c.map(|c| c.rx_bytes).unwrap_or(0),
            tx_bytes: c.map(|c| c.tx_bytes).unwrap_or(0),
            rx_rate: rx_rate.get(&info.name).copied().unwrap_or(0.0),
            tx_rate: tx_rate.get(&info.name).copied().unwrap_or(0.0),
            rx_packets: c.map(|c| c.rx_packets).unwrap_or(0),
            tx_packets: c.map(|c| c.tx_packets).unwrap_or(0),
            up: info.up,
            status: info.status.clone(),
            media: info.media.clone(),
            ipv4: info.ipv4.clone(),
            mac: info.mac.clone(),
            mtu: info.mtu,
            name: info.name.clone(),
        });
    }
    // Any interface in counters but not in ifconfig info.
    for (name, c) in &counters {
        if !seen.contains(name) {
            out.push(NetIface {
                name: name.clone(),
                rx_bytes: c.rx_bytes,
                tx_bytes: c.tx_bytes,
                rx_rate: rx_rate.get(name).copied().unwrap_or(0.0),
                tx_rate: tx_rate.get(name).copied().unwrap_or(0.0),
                rx_packets: c.rx_packets,
                tx_packets: c.tx_packets,
                up: false,
                status: String::new(),
                media: String::new(),
                ipv4: Vec::new(),
                mac: None,
                mtu: 0,
            });
        }
    }
    out
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
