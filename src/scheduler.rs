//! Centralized scheduler for FWP periodic maintenance tasks.
//!
//! A single tokio task manages all recurring jobs. Instead of polling at a
//! fixed interval, it computes the nearest next-run time across all jobs and
//! `sleep`s until exactly that moment. This gives exact precision for any
//! granularity — seconds, minutes, or days — with zero CPU usage between
//! events.
//!
//! Jobs can use either a fixed interval (`every 3600s`) or a cron expression
//! (`0 5 * * * *` = every hour at :05, UTC). Cron expressions use 6-7 fields:
//! `sec min hour day month weekday [year]`, evaluated in UTC.
//!
//! ## Adding a new periodic job
//!
//! 1. Write `fn job_xxx(state: AppState) -> BoxFuture`.
//! 2. Register it in `spawn()`:
//!    - Cron schedule:  `register_cron!("name", "0 5 * * * *", job_xxx);`
//!    - Fixed interval: `register_interval!("name", Duration::from_secs(60), job_xxx);`

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use parking_lot::Mutex;
use serde::Serialize;

use crate::cron::Cron;
use crate::error::ApiResult;
use crate::state::AppState;

type BoxFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
type JobFn = fn(AppState) -> BoxFuture;

/// How a job is triggered.
enum Trigger {
    /// Fixed interval, e.g. "every 30 seconds".
    Interval(Duration),
    /// Cron expression (5-6 fields: min hour dom month dow [sec], or with
    /// leading seconds: sec min hour dom month dow). Evaluated in UTC.
    Cron(Cron, &'static str),
}

impl Trigger {
    /// Compute the next run timestamp (Unix seconds) after `from_ts`.
    fn next_run_ts(&self, from_ts: i64) -> i64 {
        match self {
            Trigger::Interval(d) => from_ts + d.as_secs() as i64,
            Trigger::Cron(c, _) => {
                c.next_after(from_ts).unwrap_or(i64::MAX)
            }
        }
    }

    /// Human-readable schedule description for the UI.
    fn describe(&self) -> String {
        match self {
            Trigger::Interval(d) => format!("every {}s", d.as_secs()),
            Trigger::Cron(_, expr) => expr.to_string(),
        }
    }
}

struct Job {
    name: &'static str,
    trigger: Trigger,
    next_run_ts: i64,
    run: JobFn,
}

/// Per-job runtime statistics, updated by the scheduler loop.
#[derive(Debug, Default, Serialize, Clone)]
pub struct JobStat {
    pub name: &'static str,
    /// Human-readable schedule, e.g. "every 3600s" or "0 5 * * * *".
    pub schedule: String,
    pub run_count: u64,
    pub last_run_ts: Option<i64>,
    pub last_error: Option<String>,
    pub next_run_ts: Option<i64>,
}

/// Shared, snapshot-able scheduler stats — read by the API handler.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SchedulerStats {
    /// Start time of the FWP process (Unix seconds).
    pub started_at: i64,
    /// Stats for each registered job.
    pub jobs: Vec<JobStat>,
}

pub type SharedSchedulerStats = Arc<Mutex<SchedulerStats>>;

/// Spawn the scheduler with all registered periodic jobs.
pub fn spawn(state: AppState, stats: SharedSchedulerStats) {
    let now_ts = state.now_ts();
    let mut jobs: Vec<Job> = Vec::new();
    let mut job_stats: Vec<JobStat> = Vec::new();

    // Helper: register a fixed-interval job.
    macro_rules! register_interval {
        ($name:expr, $interval:expr, $initial_delay:expr, $run:expr) => {{
            let trigger = Trigger::Interval($interval);
            let desc = trigger.describe();
            let next_ts = now_ts + $initial_delay.as_secs() as i64;
            jobs.push(Job { name: $name, trigger, next_run_ts: next_ts, run: $run });
            job_stats.push(JobStat {
                name: $name,
                schedule: desc,
                run_count: 0,
                last_run_ts: None,
                last_error: None,
                next_run_ts: Some(next_ts),
            });
        }};
    }

    // Helper: register a cron-scheduled job.
    macro_rules! register_cron {
        ($name:expr, $expr:expr, $run:expr) => {{
            let cron = Cron::parse($expr)
                .unwrap_or_else(|e| panic!("invalid cron expression '{}': {e}", $expr));
            let trigger = Trigger::Cron(cron, $expr);
            let desc = trigger.describe();
            let next_ts = trigger.next_run_ts(now_ts);
            jobs.push(Job { name: $name, trigger, next_run_ts: next_ts, run: $run });
            job_stats.push(JobStat {
                name: $name,
                schedule: desc,
                run_count: 0,
                last_run_ts: None,
                last_error: None,
                next_run_ts: Some(next_ts),
            });
        }};
    }

    if state.config.monitor.enabled {
        let interval = state.config.monitor.interval_sec;
        register_interval!(
            "metric-sampling",
            Duration::from_secs(interval),
            Duration::from_secs(0),
            job_metric_sampling
        );
        register_cron!("sample-purge", "0 0 * * * *", job_sample_purge);
    }
    register_cron!("session-purge", "0 5 * * * *", job_session_purge);

    {
        let mut s = stats.lock();
        s.started_at = now_ts;
        s.jobs = job_stats;
    }

    tracing::info!(
        jobs = ?jobs.iter().map(|j| j.name).collect::<Vec<_>>(),
        "scheduler started"
    );

    tokio::spawn(async move {
        loop {
            // Find the nearest next-run across all jobs.
            let nearest = match jobs.iter().map(|j| j.next_run_ts).min() {
                Some(ts) => ts,
                None => break, // no jobs registered
            };

            // Sleep until the nearest job is due.
            let now_ts = state.now_ts();
            let delay = (nearest - now_ts).max(0) as u64;
            tokio::time::sleep(Duration::from_secs(delay)).await;

            // Wake up — run all jobs that are due.
            let now_ts = state.now_ts();
            for job in &mut jobs {
                if now_ts >= job.next_run_ts {
                    // Compute next run before executing, based on current time.
                    job.next_run_ts = job.trigger.next_run_ts(now_ts);
                    tracing::trace!(job = job.name, "scheduler: running job");
                    let run_ts = now_ts;
                    let result = (job.run)(state.clone()).await;
                    let next_ts = job.next_run_ts;
                    let mut s = stats.lock();
                    if let Some(st) = s.jobs.iter_mut().find(|st| st.name == job.name) {
                        st.run_count += 1;
                        st.last_run_ts = Some(run_ts);
                        st.next_run_ts = Some(next_ts);
                        st.last_error = result.as_ref().err().map(|e| e.to_string());
                    }
                }
            }
        }
    });
}

// ---- Job functions ----

fn job_metric_sampling(state: AppState) -> BoxFuture {
    Box::pin(async move {
        crate::monitor::sample_metrics(&state).await
    })
}

fn job_sample_purge(state: AppState) -> BoxFuture {
    Box::pin(async move {
        let retention = state.config.monitor.retention_days;
        let cutoff = state.now_ts() - (retention as i64 * 86400);
        let conn = state.db.lock().await;
        let n = crate::db::purge_old_samples(&conn, cutoff)?;
        if n > 0 {
            tracing::info!(purged = n, "old metric samples removed");
        }
        Ok(())
    })
}

fn job_session_purge(state: AppState) -> BoxFuture {
    Box::pin(async move {
        let now = state.now_ts();
        let conn = state.db.lock().await;
        crate::db::purge_expired_sessions(&conn, now)?;
        Ok(())
    })
}

// ---- API handler ----

/// GET /api/scheduler/status
///
/// Returns a snapshot of all scheduler jobs.
pub async fn status(
    State(state): State<AppState>,
) -> ApiResult<Json<SchedulerStats>> {
    let snapshot = state.scheduler_stats.lock().clone();
    Ok(Json(snapshot))
}
