# 05 — 监控采集

## 概述

后台 tokio 任务周期采样系统指标，写入 SQLite `metric_samples` 表。前端用 Chart.js 绘制时序折线图，支持时间范围选择。

## 实现细节

### 采集器 `src/monitor.rs::spawn_collector`

启动时 `main.rs` 调用，spawn 两个异步任务：

1. **采样任务**：`tokio::time::interval(interval_sec)` 每 30 秒唤醒，调用 `sample_metrics()` 采集并写入
   - 首次调用立即执行（prime CPU delta），保证第一次存储的数据有效
   - `MissedTickBehavior::Skip`：暂停后不补积压的 tick
2. **清理任务**：每小时执行 `purge_old_samples(before_ts)`，删除超过 `retention_days`（默认 30 天）的样本

### 采样逻辑 `collect_samples(now)`

每次采集生成一批 `MetricSample`（同一时间戳）：

| 分类 | 名称 | 值 | 来源 |
|---|---|---|---|
| **cpu** | `total` | 总体使用率 % | `kern.cp_times` delta |
| **cpu** | `core0..coreN` | 每核使用率 % | 同上 |
| **cpu** | `freq` | CPU 频率 MHz | `dev.cpu.0.freq` |
| **memory** | `usage` | 使用率 % | `vm.stats.vm.*` |
| **memory** | `used` | 已用字节 | `ps × (active + wire)` |
| **memory** | `free` | 空闲字节 | `ps × (free + inactive + cache)` |
| **memory** | `wired` | Wired 字节 | `ps × wire` |
| **memory** | `total` | 总内存字节 | `ps × page_count` |
| **load** | `1` / `5` / `15` | load average | `uptime` |
| **temp** | `cpu0..cpuN` | 温度 °C | `dev.cpu.N.temperature` |

CPU delta 使用 `MONITOR_CPU`（独立的 `LazyLock<Mutex<Option<CpuState>>>`），与仪表盘的 `LAST_CP_TIMES` 隔离。

### 写入

`db::insert_samples(conn, &samples)` — 单事务批量 `INSERT OR REPLACE`，幂等（相同 ts+category+name 覆盖）。

### 查询 API

`GET /api/monitor/series?category=&name=&from=&to=`
- 返回 `[{ts, value}, ...]` 按时间升序
- 利用索引 `idx_samples_query (category, name, ts)` 快速范围扫描

`GET /api/monitor/latest`
- 每个分类返回各 name 的最新采样值
- 子查询 `SELECT name, MAX(ts) ... GROUP BY name` 再 JOIN

### 前端图表 `web/js/pages/monitor.js`

三个页面：
- **CPU & 负载**（`/monitor`）：CPU 总体使用率 + load average（1/5/15 三线）
- **内存**（`/monitor/memory`）：使用率 % + 用量字节（used/wired）
- **温度**（`/monitor/temp`）：各核心温度（多色线，名称从 `/api/monitor/latest` 动态发现）

**Chart.js 加载**：`loadChartJs()` 动态插入 `<script>` 标签加载 UMD 全局（Chart.js + date-fns 适配器），加载后缓存 Promise。

**时间范围**：按钮组（1h/6h/24h/7d/30d），点击后计算 `from = now - range`，重新查询并重绘。Chart 实例存 `CHARTS` map，切换时 `destroy()` 旧实例。

**图表选项**：时间 X 轴（`type: 'time'`）、深色主题配色、tooltip 回调（字节格式化）、`interaction: { mode: 'index' }`。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/monitor/series?category=&name=&from=&to=` | 时序查询 |
| GET | `/api/monitor/latest` | 各分类最新值 |

## 配置项

```toml
[monitor]
enabled = true          # 是否启动采集器
interval_sec = 30       # 采样间隔（秒）
retention_days = 30     # 数据保留天数
```

## 外部依赖

- 系统命令：`/sbin/sysctl`、`/usr/bin/uptime`
- crate：`tokio`（interval/spawn）、`parking_lot`、`std::sync::LazyLock`
- 前端：Chart.js 4.4.7（UMD 本地托管）、chartjs-adapter-date-fns 3.0.0

## 已知限制

- 未采集磁盘 I/O 和网络流量（后续扩展）
- 无降采样（长期数据 30s 粒度，7 天约 2 万点，前端通过 `pointRadius: 0` 处理密集数据）
- 无告警规则和通知（设计已完成，待实现）
- `sysctl -aN` 缓存后不刷新（热插拔 CPU 场景需重启服务）
