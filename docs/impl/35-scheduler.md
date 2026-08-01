# 35 — 中央调度器

## 概述

FWP 所有周期性维护任务（监控采集、样本清理、Session 清理等）由一个中央调度器统一管理。调度器是一个单 tokio 任务，使用 sleep-until-next 模式实现精确调度，零 CPU 空转。内置轻量 cron 表达式解析器（`src/cron.rs`），不依赖任何外部调度框架。

## 实现细节

### 调度循环 `src/scheduler.rs::spawn`

调度器启动时注册所有 job，然后进入主循环：

```
loop {
    nearest = min(job.next_run_ts for all jobs)   // 找最近的
    sleep(nearest - now)                           // 精确睡眠到那个时刻
    for job in due_jobs { run(job) }               // 醒来执行到期任务
}
```

- **sleep-until-next**：不轮询、不固定 tick，计算所有 job 中最近的下次执行时间，`tokio::time::sleep` 到那个精确时刻再醒来。秒级和天级 job 精度相同（秒级），两次执行之间 CPU 占用为零。
- **串行执行**：所有 job 在同一个 task 内按序执行。当前 job 都很轻（单条 SQL），不会互相阻塞。如果未来加入耗时任务，需考虑在 job 内部 `tokio::spawn` 隔离。

### 触发方式 `Trigger`

```rust
enum Trigger {
    Interval(Duration),          // 固定间隔，如 every 30s
    Cron(Cron, &'static str),    // cron 表达式 + 原始字符串（用于展示）
}
```

两种注册宏：

- `register_interval!("name", Duration::from_secs(30), Duration::from_secs(0), job_fn)` — 固定间隔，第三参数为初始延迟
- `register_cron!("name", "0 5 * * * *", job_fn)` — cron 表达式

### 已注册的任务

| 名称 | 触发方式 | 说明 |
|---|---|---|
| `metric-sampling` | `every {interval_sec}s`（默认 30s，首次立即） | 系统指标采样，调用 `monitor::sample_metrics()` |
| `sample-purge` | cron `0 0 * * * *`（每小时整点） | 删除超过 `retention_days` 的监控样本 |
| `session-purge` | cron `0 5 * * * *`（每小时 :05） | 删除过期 Session |

### 运行时统计 `SchedulerStats` / `JobStat`

每个 job 的运行状态实时记录在共享的 `SharedSchedulerStats`（`Arc<Mutex<SchedulerStats>>`）中：

```rust
struct JobStat {
    name: &'static str,
    schedule: String,          // "every 30s" 或 "0 5 * * * *"
    run_count: u64,            // 自启动以来执行次数
    last_run_ts: Option<i64>,  // 上次执行 Unix 时间戳
    last_error: Option<String>,// 上次错误信息
    next_run_ts: Option<i64>,  // 下次预计执行
}
```

通过 `GET /api/scheduler/status` 暴露给前端，在"面板状态"页面的"定时任务"表格中展示。

### cron 表达式解析器 `src/cron.rs`

自写的轻量解析器（~130 行），零外部依赖（仅用 chrono）。

**支持语法：**
- 5 字段：`min hour dom month dow`（秒默认 0）
- 6 字段：`sec min hour dom month dow`
- `*`（通配）、纯数字（`5`）、范围（`1-10`）、步进（`*/15`、`2-10/2`）、列表（`1,5,10`）
- 纯数字，不支持命名日/月（MON/JAN）

**匹配方式：** `next_after(from_ts)` 从 `from_ts + 1` 秒开始逐秒搜索（用 `Local` 时区提取时分秒日月周与 cron 字段匹配），最多搜索 366 天。

**时区：** 使用系统本地时区（`chrono::Local`），`0 3 * * *` 即本地时间凌晨 3 点。

**为何不用外部库：**
- `cron` 0.17 — 用 `phf`（perfect hash）映射 MON/TUE/JAN 名称，拉入 phf + siphasher + rand 等 8 个 crate，FWP 不需要名称解析
- `croner` 3.0 — 用 `derive_builder` + `darling` + `strum` 生成 builder pattern，拉入 12 个 crate，更重
- 自写解析器只需 chrono（已引入），功能覆盖 FWP 所有需求

### AppState 集成

```rust
// state.rs
pub struct AppState {
    ...
    pub scheduler_stats: SharedSchedulerStats,
}
```

`main.rs` 中初始化为 `Default::default()`，传入 `scheduler::spawn(state, state.scheduler_stats.clone())`。

## API

| Method | Path | 说明 |
|---|---|---|
| **GET** | `/api/scheduler/status` | 返回调度器状态快照：`{ started_at, jobs: [JobStat] }` |

## 外部依赖

- crate：`chrono`（时间计算 + Local 时区）、`parking_lot`（Mutex）、`tokio`（sleep/spawn）
- 无 cron/croner/tokio-cron-scheduler 等外部调度框架

## 配置项

调度器自身无可配置项。相关配置来自 `[monitor]` 段：

```toml
[monitor]
enabled = true          # false 时不注册 metric-sampling 和 sample-purge
interval_sec = 30       # metric-sampling 的间隔
retention_days = 30     # sample-purge 的保留天数
```

## 已知限制

- **串行执行**：所有 job 在同一 task 内按序运行。当前 job 都很轻，不影响。如果未来加入耗时任务，需在 job 内部 `tokio::spawn` 隔离
- **无动态增删**：job 在 `spawn()` 时硬编码注册，运行时不可动态添加/移除/修改间隔
- **无持久化**：job 定义在源码中，重启后执行计数归零
- **无重试**：job 失败只记录 error，不自动重试
- **cron 功能有限**：不支持 L/W/# 修饰符、命名日/月、时区指定
- **逐秒搜索**：`next_after` 用逐秒遍历而非字段级跳跃，对于极远时间点（如一年后）理论上慢，但由于 sleep-until-next 模式每次只算下一次，实际无性能影响
