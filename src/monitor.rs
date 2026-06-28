//! Background metric collector + monitoring query handlers.
//!
//! A tokio task wakes every `interval_sec`, reads system metrics (CPU, memory,
//! load, temperature) via sysctl, and writes a batch of samples to SQLite.
//! A separate purge task trims samples older than `retention_days`.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::{self, MetricSample};
use crate::error::ApiResult;
use crate::state::AppState;

// ---- Collector ----

/// Spawn the background collector task. Returns immediately.
pub fn spawn_collector(state: AppState) {
    if !state.config.monitor.enabled {
        tracing::info!("monitoring disabled by config");
        return;
    }
    let interval = state.config.monitor.interval_sec;
    let retention = state.config.monitor.retention_days;
    let s = state.clone();
    tokio::spawn(async move {
        // Prime the CPU delta on first tick so the first stored sample is real.
        let _ = sample_metrics(&s).await;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval));
        // Don't accumulate missed ticks after a pause.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(e) = sample_metrics(&s).await {
                tracing::warn!(error = %e, "metric sampling failed");
            }
        }
    });

    // Purge task — runs hourly.
    let s2 = state.clone();
    tokio::spawn(async move {
        let mut hour = tokio::time::interval(std::time::Duration::from_secs(3600));
        hour.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            hour.tick().await;
            let cutoff = s2.now_ts() - (retention as i64 * 86400);
            let conn = s2.db.lock().await;
            match db::purge_old_samples(&conn, cutoff) {
                Ok(n) if n > 0 => tracing::info!(purged = n, "old metric samples removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "purge old samples failed"),
            }
        }
    });
}

/// Read current metrics and write a batch of samples.
async fn sample_metrics(state: &AppState) -> anyhow::Result<()> {
    let now = state.now_ts();
    let samples = collect_samples(now)?;

    let conn = state.db.lock().await;
    db::insert_samples(&conn, &samples)?;
    Ok(())
}

/// Gather all metric samples for a single point in time.
fn collect_samples(now: i64) -> anyhow::Result<Vec<MetricSample>> {
    let mut out = Vec::with_capacity(32);

    // CPU usage — requires delta against previous sample. We reuse a dedicated
    // collector-level static so monitoring and live-metrics endpoints don't
    // interfere with each other's deltas.
    let (cpu_total, per_core) = cpu_usage_delta();
    out.push(MetricSample {
        ts: now,
        category: "cpu".into(),
        name: "total".into(),
        value: cpu_total as f64,
    });
    for (i, pct) in per_core.iter().enumerate() {
        out.push(MetricSample {
            ts: now,
            category: "cpu".into(),
            name: format!("core{i}"),
            value: *pct as f64,
        });
    }

    // CPU frequency.
    if let Some(freq) = read_sysctl_f64("dev.cpu.0.freq") {
        out.push(MetricSample {
            ts: now,
            category: "cpu".into(),
            name: "freq".into(),
            value: freq,
        });
    }

    // Memory.
    let ps = read_sysctl_f64("vm.stats.vm.v_page_size").unwrap_or(4096.0);
    let vpc = |n: &str| read_sysctl_f64(n).unwrap_or(0.0);
    let total_pages = vpc("vm.stats.vm.v_page_count");
    let active = vpc("vm.stats.vm.v_active_count");
    let wire = vpc("vm.stats.vm.v_wire_count");
    let free = vpc("vm.stats.vm.v_free_count");
    let inactive = vpc("vm.stats.vm.v_inactive_count");
    let cache = vpc("vm.stats.vm.v_cache_count");
    let mem_total = ps * total_pages;
    let mem_used = ps * (active + wire);
    let mem_usage = if mem_total > 0.0 { mem_used / mem_total * 100.0 } else { 0.0 };
    out.push(MetricSample { ts: now, category: "memory".into(), name: "usage".into(), value: mem_usage });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "used".into(), value: mem_used });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "free".into(), value: ps * (free + inactive + cache) });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "wired".into(), value: ps * wire });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "total".into(), value: mem_total });

    // Load average.
    let la = read_loadavg();
    out.push(MetricSample { ts: now, category: "load".into(), name: "1".into(), value: la[0] });
    out.push(MetricSample { ts: now, category: "load".into(), name: "5".into(), value: la[1] });
    out.push(MetricSample { ts: now, category: "load".into(), name: "15".into(), value: la[2] });

    // Temperatures.
    for (name, value) in read_temps() {
        out.push(MetricSample { ts: now, category: "temp".into(), name, value: value as f64 });
    }

    Ok(out)
}

// ---- CPU delta (monitoring-local) ----

use parking_lot::Mutex;
use std::sync::LazyLock;

struct CpuState {
    times: Vec<u64>,
}

static MONITOR_CPU: LazyLock<Mutex<Option<CpuState>>> = LazyLock::new(|| Mutex::new(None));

fn cpu_usage_delta() -> (f32, Vec<f32>) {
    let raw = read_sysctl("kern.cp_times").unwrap_or_default();
    let times: Vec<u64> = raw
        .split_whitespace()
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let ncpu = (times.len() / 5).max(1);
    let mut guard = MONITOR_CPU.lock();
    let mut per: Vec<f32> = Vec::with_capacity(ncpu);
    let mut sum_busy = 0u64;
    let mut sum_total = 0u64;
    if let Some(prev) = guard.as_ref() {
        if prev.times.len() == times.len() {
            for i in 0..ncpu {
                let off = i * 5;
                let total = (0..5).map(|j| times[off + j].saturating_sub(prev.times[off + j])).sum::<u64>();
                let busy = (0..4).map(|j| times[off + j].saturating_sub(prev.times[off + j])).sum::<u64>();
                per.push(if total > 0 { busy as f32 / total as f32 * 100.0 } else { 0.0 });
                sum_busy += busy;
                sum_total += total;
            }
        }
    }
    *guard = Some(CpuState { times });
    let overall = if sum_total > 0 { sum_busy as f32 / sum_total as f32 * 100.0 } else { 0.0 };
    if per.is_empty() { per = vec![0.0; ncpu]; }
    (overall, per)
}

// ---- API handlers ----

#[derive(Debug, Deserialize)]
pub struct SeriesQuery {
    pub category: String,
    pub name: String,
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Serialize)]
pub struct SeriesResponse {
    pub category: String,
    pub name: String,
    pub points: Vec<(i64, f64)>,
}

pub async fn series(
    State(state): State<AppState>,
    Query(q): Query<SeriesQuery>,
) -> ApiResult<Json<SeriesResponse>> {
    let conn = state.db.lock().await;
    let samples = db::query_series(&conn, &q.category, &q.name, q.from, q.to)?;
    let points: Vec<(i64, f64)> = samples.into_iter().map(|s| (s.ts, s.value)).collect();
    Ok(Json(SeriesResponse {
        category: q.category,
        name: q.name,
        points,
    }))
}

#[derive(Debug, Serialize)]
pub struct LatestResponse {
    pub cpu: Vec<MetricSample>,
    pub memory: Vec<MetricSample>,
    pub load: Vec<MetricSample>,
    pub temp: Vec<MetricSample>,
}

pub async fn latest(State(state): State<AppState>) -> ApiResult<Json<LatestResponse>> {
    let conn = state.db.lock().await;
    Ok(Json(LatestResponse {
        cpu: db::latest_in_category(&conn, "cpu")?,
        memory: db::latest_in_category(&conn, "memory")?,
        load: db::latest_in_category(&conn, "load")?,
        temp: db::latest_in_category(&conn, "temp")?,
    }))
}

// ---- Sysctl helpers ----

fn read_sysctl(name: &str) -> std::io::Result<String> {
    let out = std::process::Command::new("/sbin/sysctl")
        .arg("-n")
        .arg(name)
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn read_sysctl_f64(name: &str) -> Option<f64> {
    read_sysctl(name).ok().and_then(|s| s.trim().parse().ok())
}

fn read_loadavg() -> [f64; 3] {
    let mut la = [0.0_f64; 3];
    if let Ok(out) = std::process::Command::new("/usr/bin/uptime").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        let nums: Vec<f64> = s
            .split_whitespace()
            .filter_map(|t| t.trim_matches(|c: char| !c.is_ascii_digit() && c != '.').parse().ok())
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

fn read_temps() -> Vec<(String, f32)> {
    let names = sysctl_names_matching(|n| n.starts_with("dev.cpu.") && n.ends_with(".temperature"));
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if let Ok(raw) = read_sysctl(&name) {
            let num: String = raw.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Ok(v) = num.parse::<f32>() {
                let core = name
                    .strip_prefix("dev.cpu.")
                    .and_then(|s| s.split('.').next())
                    .unwrap_or("?");
                out.push((format!("cpu{core}"), v));
            }
        }
    }
    out
}

fn sysctl_names_matching<F: Fn(&str) -> bool>(pred: F) -> Vec<String> {
    static ALL_NAMES: LazyLock<Mutex<Option<Vec<String>>>> =
        LazyLock::new(|| Mutex::new(None));
    let names = {
        let mut g = ALL_NAMES.lock();
        if g.is_none() {
            *g = Some(read_all_sysctl_names());
        }
        g.as_ref().unwrap().clone()
    };
    names.into_iter().filter(|n| pred(n)).collect()
}

fn read_all_sysctl_names() -> Vec<String> {
    let out = std::process::Command::new("/sbin/sysctl").arg("-aN").output();
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
