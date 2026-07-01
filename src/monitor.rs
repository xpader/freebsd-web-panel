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
use crate::sysinfo;

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
    if let Some(freq) = sysinfo::read_f64("dev.cpu.0.freq") {
        out.push(MetricSample {
            ts: now,
            category: "cpu".into(),
            name: "freq".into(),
            value: freq,
        });
    }

    // Memory.
    let ps = sysinfo::read_f64("vm.stats.vm.v_page_size").unwrap_or(4096.0);
    let vpc = |n: &str| sysinfo::read_f64(n).unwrap_or(0.0);
    let total_pages = vpc("vm.stats.vm.v_page_count");
    let active = vpc("vm.stats.vm.v_active_count");
    let wire = vpc("vm.stats.vm.v_wire_count");
    let free = vpc("vm.stats.vm.v_free_count");
    let inactive = vpc("vm.stats.vm.v_inactive_count");
    let laundry = vpc("vm.stats.vm.v_laundry_count");
    let cache = vpc("vm.stats.vm.v_cache_count");
    let mem_total = ps * total_pages;
    let mem_used = ps * (active + wire);
    let mem_usage = if mem_total > 0.0 { mem_used / mem_total * 100.0 } else { 0.0 };
    out.push(MetricSample { ts: now, category: "memory".into(), name: "usage".into(), value: mem_usage });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "used".into(), value: mem_used });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "total".into(), value: mem_total });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "active".into(), value: ps * active });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "wired".into(), value: ps * wire });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "inactive".into(), value: ps * inactive });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "laundry".into(), value: ps * laundry });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "cache".into(), value: ps * cache });
    out.push(MetricSample { ts: now, category: "memory".into(), name: "free".into(), value: ps * free });

    // Load average.
    let la = sysinfo::read_loadavg();
    out.push(MetricSample { ts: now, category: "load".into(), name: "1".into(), value: la[0] });
    out.push(MetricSample { ts: now, category: "load".into(), name: "5".into(), value: la[1] });
    out.push(MetricSample { ts: now, category: "load".into(), name: "15".into(), value: la[2] });

    // Temperatures.
    let ncpu = sysinfo::read_u64("hw.ncpu").unwrap_or(1) as u32;
    for (i, value) in sysinfo::read_core_temps(ncpu) {
        out.push(MetricSample { ts: now, category: "temp".into(), name: format!("cpu{i}"), value: value as f64 });
    }

    // Network: rate (bytes/sec) for live charts + traffic (bytes per interval)
    // for accurate aggregation.
    let net = net_rate_delta(now);
    for (name, (rx_rate, tx_rate), rx_bytes, tx_bytes) in &net {
        out.push(MetricSample { ts: now, category: "net".into(), name: format!("{name}.rx"), value: *rx_rate });
        out.push(MetricSample { ts: now, category: "net".into(), name: format!("{name}.tx"), value: *tx_rate });
        out.push(MetricSample { ts: now, category: "net_bytes".into(), name: format!("{name}.rx"), value: *rx_bytes as f64 });
        out.push(MetricSample { ts: now, category: "net_bytes".into(), name: format!("{name}.tx"), value: *tx_bytes as f64 });
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
    let times: Vec<u64> = sysinfo::read_cp_times();
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

// ---- Network rate delta (monitoring-local) ----

use crate::sysinfo::NetCounters;
use std::collections::HashMap;

struct NetState {
    ts: i64,
    counters: HashMap<String, NetCounters>,
}

static MONITOR_NET: LazyLock<Mutex<Option<NetState>>> = LazyLock::new(|| Mutex::new(None));

/// Compute per-interface RX/TX rates (bytes/sec) and actual bytes transferred
/// since the last sample.  Both are derived from the delta of cumulative
/// counters — rate = delta / dt, traffic = delta itself.
/// Uses a monitoring-local static so it doesn't interfere with the
/// live-metrics endpoint's own delta tracking.
fn net_rate_delta(now: i64) -> Vec<(String, (f64, f64), u64, u64)> {
    let counters = sysinfo::read_net_counters();
    let mut guard = MONITOR_NET.lock();
    let mut out = Vec::with_capacity(counters.len());
    if let Some(ref prev) = *guard {
        let dt = (now - prev.ts).max(1) as f64;
        for (name, cur) in &counters {
            if let Some(p) = prev.counters.get(name) {
                let rx_delta = cur.rx_bytes.saturating_sub(p.rx_bytes);
                let tx_delta = cur.tx_bytes.saturating_sub(p.tx_bytes);
                let rx_rate = rx_delta as f64 / dt;
                let tx_rate = tx_delta as f64 / dt;
                out.push((name.clone(), (rx_rate, tx_rate), rx_delta, tx_delta));
            }
        }
    }
    *guard = Some(NetState { ts: now, counters });
    out
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

/// Aggregated series for network traffic totals.  Uses cumulative byte
/// counters (`net_bytes` category) and computes MAX-MIN per bucket for
/// exact bytes transferred — not interpolated from instantaneous rates.
#[derive(Debug, Deserialize)]
pub struct AggregateQuery {
    pub category: String,
    pub name: String,
    pub from: i64,
    pub to: i64,
    pub bucket: i64, // bucket size in seconds
}

pub async fn aggregate(
    State(state): State<AppState>,
    Query(q): Query<AggregateQuery>,
) -> ApiResult<Json<SeriesResponse>> {
    let conn = state.db.lock().await;
    let buckets = db::query_counter_aggregate(
        &conn,
        "net_bytes",
        &q.name,
        q.from,
        q.to,
        q.bucket,
    )?;
    let points: Vec<(i64, f64)> = buckets.into_iter().collect();
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
    pub net: Vec<MetricSample>,
}

pub async fn latest(State(state): State<AppState>) -> ApiResult<Json<LatestResponse>> {
    let conn = state.db.lock().await;
    Ok(Json(LatestResponse {
        cpu: db::latest_in_category(&conn, "cpu")?,
        memory: db::latest_in_category(&conn, "memory")?,
        load: db::latest_in_category(&conn, "load")?,
        temp: db::latest_in_category(&conn, "temp")?,
        net: db::latest_in_category(&conn, "net")?,
    }))
}
