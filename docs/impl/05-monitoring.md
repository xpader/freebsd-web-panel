# 05 — 监控采集

## 概述

系统指标由中央调度器（[35-scheduler.md](35-scheduler.md)）周期采样，写入 SQLite `metric_samples` 表。前端用 Chart.js 绘制时序折线图，支持时间范围选择。

## 实现细节

### 采集器 `src/monitor.rs::sample_metrics`

监控采集不再拥有独立的 tokio 任务，而是作为调度器的一个 interval job（`metric-sampling`）注册在 `scheduler.rs` 中。采样逻辑保持不变：

- 每 `interval_sec`（默认 30 秒）由调度器唤醒调用 `sample_metrics()`
- 首次执行在启动时立即运行（initial_delay = 0），prime CPU delta 保证第一次存储的数据有效

### 采样逻辑 `collect_samples(now)`

每次采集调用 `src/sysinfo.rs` 中的读取器（sysctl(3) 系统调用，不走子进程）生成一批 `MetricSample`（同一时间戳）：

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
| **load** | `1` / `5` / `15` | load average | `getloadavg(3)` |
| **temp** | `cpu0..cpuN` | 温度 °C | `dev.cpu.N.temperature`（`CtlValue::Temperature`） |
| **net** | `{iface}.rx` / `{iface}.tx` | 收发速率 bytes/sec | `netstat` 计数器 delta（见下） |
| **net_bytes** | `{iface}.rx` / `{iface}.tx` | 每区间收发字节数 | 同上 delta 的原始字节差值（用于流量 SUM 聚合） |

CPU delta 使用 `MONITOR_CPU`（独立的 `LazyLock<Mutex<Option<CpuState>>>`），与仪表盘的 `LAST_CP_TIMES` 隔离。

### 网络流量速率 delta

`net_rate_delta(now)` 计算各接口收发速率（bytes/sec），逻辑同 CPU delta：
1. `sysinfo::read_net_counters()` 取当前累计计数器（仅排除噪音伪接口；接口名已剥离 `*` 后缀）
2. 与 `MONITOR_NET`（`LazyLock<Mutex<Option<NetState>>>`）中上次计数器+时间戳做差：`rate = (cur - prev) / (now - prev_ts)`
3. 每个接口生成两条采样：`{iface}.rx`（下载）、`{iface}.tx`（上传），分类 `net`

噪音过滤由 `sysinfo::read_net_counters()` 内部的 `is_noise_iface()` 完成（见 [13-sysinfo.md](13-sysinfo.md)），采集器自动继承。在此基础上 `net_rate_delta()` 再做一次 **UP + IP 过滤**（与仪表盘 `collect_network` 一致）：只有当前 `IFF_UP` 置位、且至少拥有一个 IPv4 或全局 IPv6 地址的接口才会写入 `net`/`net_bytes` 采样。bridge、tap、lagg 成员、jail 宿主侧 epair*a 等 UP 但无 IP 的口，以及 DHCP 续约失败/手动 `ifconfig down` 后残留 IP 配置但已 DOWN 的口，都不再进入监控时序库——它们仍可在"网络"页实时查看。

`MONITOR_NET` 与仪表盘的 `LAST_NET` 独立，避免互相干扰 delta。

> **已知问题**：
> - **`*` 后缀**：`netstat -i` 输出中接口名后的 `*` 表示该接口未 UP（如 `bge0*`）。早期 `read_net_counters()` 未剥离该后缀，导致 DB 同时存在 `bge0*` 与 `bge0` 两套序列，前端画出重复曲线。`read_net_counters()` 已修正为 `trim_end_matches('*')`。

### 写入

`db::insert_samples(conn, &samples)` — 单事务批量 `INSERT OR REPLACE`，幂等（相同 ts+category+name 覆盖）。





## API

| Method | Path | 说明 |
|---|---|---|
| **GET** | `/api/monitor/series?category=&from=&to=&names=a,b` | 原始采样点。`names` 为逗号分隔的序列名列表，返回 `{ series: { name: [[ts, value], ...] } }`。一次调用取回同一分类下多个序列 |
| **GET** | `/api/monitor/grouped?category=&from=&to=&bucket=&agg=&names=a,b` | 按时间桶聚合后的瞬时值。`agg` 可选 `min`/`avg`/`max` |
| **GET** | `/api/monitor/aggregate?category=&from=&to=&bucket=&names=a,b` | 累计计数器做 SUM（每个 bucket 内累加已存好的 delta），得到区间内真实字节数。当前唯一的 counter 分类是 `net_bytes` |
| GET | `/api/monitor/latest` | 每个 (category,name) 的最新一条采样，供前端发现接口/指标使用 |

三类端点都返回 `{ series: { name: points } }`，前端一次调用即可拿到一张图表所需的全部数据集。`names` 参数使用 Serde `comma_list` 反序列化，将逗号分隔的字符串解析为 `Vec<String>`。



## 配置项

```toml
[monitor]
enabled = true          # 是否启动采集器
interval_sec = 30       # 采样间隔（秒）
retention_days = 30     # 数据保留天数
```

## 外部依赖

- crate：`sysctl`、`libc`（通过 `src/sysinfo.rs`，详见 [13-sysinfo.md](13-sysinfo.md)）、`parking_lot`、`std::sync::LazyLock`
- 调度器：详见 [35-scheduler.md](35-scheduler.md)
- 前端：Chart.js 4.4.7（UMD 本地托管）、chartjs-adapter-date-fns 3.0.0

## 已知限制

- 未采集磁盘 I/O（后续扩展）
- 降采样为实时 SQL 聚合，无预聚合表（长时间范围 + 大桶时查询仍需扫描全部原始行，但返回点数大幅减少，前端渲染流畅）
- 无告警规则和通知（设计已完成，待实现）
- 网络流量仅记录每区间字节数（`net_bytes`），接口增减时历史曲线会断缺
