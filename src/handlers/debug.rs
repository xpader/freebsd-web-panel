//! Debug/diagnostic endpoints — exposed under `/api/debug/` for operators.
//!
//! Currently only one endpoint:
//!
//! * `GET /api/debug/jemalloc-stats` — returns live jemalloc allocator stats
//!   (allocated / active / metadata / resident / mapped / retained) plus
//!   OS-reported process RSS via `sysctl(KERN_PROC_PID)`. Used by the
//!   "FWP 状态" page under Monitor to diagnose memory growth.

use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

extern "C" {
    fn mallctl(
        name: *const libc::c_char,
        oldp: *mut libc::c_void,
        oldlenp: *mut usize,
        newp: *mut libc::c_void,
        newlen: usize,
    ) -> libc::c_int;
}

/// Advance jemalloc's epoch counter so subsequent `stats.*` reads reflect
/// the current state (jemalloc lazily aggregates stats across arenas).
fn bump_epoch() {
    let mut epoch: u64 = 1;
    let mut epoch_len = std::mem::size_of::<u64>();
    unsafe {
        let _ = mallctl(
            b"epoch\0".as_ptr() as *const libc::c_char,
            &mut epoch as *mut _ as *mut libc::c_void,
            &mut epoch_len,
            &mut epoch as *mut _ as *mut libc::c_void,
            std::mem::size_of::<u64>(),
        );
    }
}

/// Read a `usize`-typed mallctl stat. Returns `None` if the key is missing
/// or the read fails — callers treat `None` as "unknown" rather than 0.
fn read_stat(key: &[u8]) -> Option<usize> {
    let mut val: usize = 0;
    let mut len = std::mem::size_of::<usize>();
    let rc = unsafe {
        mallctl(
            key.as_ptr() as *const libc::c_char,
            &mut val as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 { Some(val) } else { None }
}

#[derive(Debug, Serialize)]
pub struct JemallocStats {
    /// Bytes the application currently holds (live Rust objects).
    pub allocated: Option<usize>,
    /// Bytes in pages jemalloc is actively using (allocated + internal fragmentation).
    pub active: Option<usize>,
    /// Bytes jemalloc uses for its own metadata (arena headers, bitmaps, etc.).
    pub metadata: Option<usize>,
    /// Bytes jemalloc has mapped and still owns (jemalloc's contribution to RSS).
    /// Includes dirty + muzzy pages awaiting decay back to the OS.
    pub resident: Option<usize>,
    /// Total bytes jemalloc has mmap'd (active + retained + metadata).
    pub mapped: Option<usize>,
    /// Bytes jemalloc has kept in its address space but returned to the OS
    /// (madvise'd). Not counted in RSS.
    pub retained: Option<usize>,
    /// Total process RSS as reported by the OS (includes code segment, debug
    /// symbols, stack, mmap'd libraries — everything jemalloc doesn't manage).
    pub process_rss: Option<usize>,
}

/// Read process RSS via `sysctl(KERN_PROC_PID)`. The `kinfo_proc` struct
/// has `ki_rssize` (segsz_t = int64 on amd64, in pages) at byte offset 264
/// on FreeBSD 15.
fn get_process_rss() -> Option<usize> {
    let pid = std::process::id() as libc::c_int;
    let mut mib: [libc::c_int; 4] = [1, 14, 1, pid]; // CTL_KERN, KERN_PROC, KERN_PROC_PID
    let mut buf = vec![0u8; 4096];
    let mut len = buf.len();
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            4,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len < 272 {
        return None;
    }
    let rsize_pages = i64::from_ne_bytes(buf[264..272].try_into().ok()?);
    if rsize_pages <= 0 {
        return None;
    }
    Some((rsize_pages as usize) * unsafe { libc::getpagesize() as usize })
}

/// GET /api/debug/jemalloc-stats
pub async fn jemalloc_stats() -> ApiResult<Json<JemallocStats>> {
    bump_epoch();
    Ok(Json(JemallocStats {
        allocated:   read_stat(b"stats.allocated\0"),
        active:      read_stat(b"stats.active\0"),
        metadata:    read_stat(b"stats.metadata\0"),
        resident:    read_stat(b"stats.resident\0"),
        mapped:      read_stat(b"stats.mapped\0"),
        retained:    read_stat(b"stats.retained\0"),
        process_rss: get_process_rss(),
    }))
}

/// Snapshot of tokio runtime metrics, read on-demand from `Handle::metrics()`.
#[derive(Debug, Serialize)]
pub struct TokioMetrics {
    pub workers_count: usize,
    pub live_tasks_count: usize,
    pub total_polls_count: u64,
    pub total_busy_duration_ms: u64,
    pub global_queue_depth: usize,
    pub total_local_queue_depth: usize,
    pub total_steal_count: u64,
    pub blocking_queue_depth: usize,
    pub blocking_threads_count: usize,
}

/// GET /api/debug/tokio-metrics
pub async fn tokio_metrics() -> ApiResult<Json<TokioMetrics>> {
    let m = tokio::runtime::Handle::current().metrics();
    let workers = m.num_workers();
    let mut total_polls = 0u64;
    let mut total_busy_ms = 0u64;
    let mut total_local_queue = 0usize;
    let mut total_steals = 0u64;
    for i in 0..workers {
        total_polls += m.worker_poll_count(i);
        total_busy_ms += m.worker_total_busy_duration(i).as_millis() as u64;
        total_local_queue += m.worker_local_queue_depth(i);
        total_steals += m.worker_steal_count(i);
    }
    Ok(Json(TokioMetrics {
        workers_count: workers,
        live_tasks_count: m.num_alive_tasks(),
        total_polls_count: total_polls,
        total_busy_duration_ms: total_busy_ms,
        global_queue_depth: m.global_queue_depth(),
        total_local_queue_depth: total_local_queue,
        total_steal_count: total_steals,
        blocking_queue_depth: m.blocking_queue_depth(),
        blocking_threads_count: m.num_blocking_threads(),
    }))
}
