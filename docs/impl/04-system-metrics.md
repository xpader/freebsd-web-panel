# 04 — 系统指标（实时）

## 概述

仪表盘使用的实时系统指标端点。每次请求即时采样，CPU 使用率通过两次 `kern.cp_times` 采样差值计算。区别于监控模块（周期性持久化历史数据）。

## 实现细节

### 静态信息 `GET /api/system/info`

`handlers/system.rs::system_info` — 一次性读取，不涉及采样：

| 指标 | sysctl 来源 |
|---|---|
| hostname | `kern.hostname` |
| os_release | `kern.osrelease` |
| os_version | `kern.osreldate` |
| kernel | `kern.ident` |
| cpu_model | `hw.model` |
| cpu_cores | `hw.ncpu` |
| memory_total | `hw.physmem` |
| swap_total | `swapinfo -k` 求和 |
| boot_time | `kern.boottime`（解析 `{ sec = N, ... }`） |
| uptime | `now - boot_time` |
| loadavg | `uptime` 输出末尾三数 |

### 实时指标 `GET /api/system/metrics`

`handlers/system.rs::system_metrics` — 每次请求采样：

**CPU 使用率（delta 计算）**：
1. 读 `kern.cp_times`：每核 5 个值（user/nice/sys/intr/idle），共 `ncpu × 5` 个数字
2. 与 `LAST_CP_TIMES`（`LazyLock<Mutex<Option<CpuSample>>>`）中上次采样做差
3. `busy = user+nice+sys+intr`，`total = busy+idle`
4. 使用率 = `busy/total × 100`
5. 存储当前采样供下次 delta

关键：`LAST_CP_TIMES` 是 **本端点专用**的全局静态，与监控模块（`monitor.rs::MONITOR_CPU`）独立，避免互相干扰 delta。

**内存**（`vm.stats.vm.*`）：

```
page_size × (active + wire) = used
page_size × (free + inactive + cache) = free
usage = used / total × 100
```

**Swap**：`/usr/sbin/swapinfo -k` 解析（跳过表头，1K-blocks → bytes）。

**温度**：`sysctl -aN` 缓存后过滤 `dev.cpu.N.temperature`，值格式 `"44.0C"`，取数字部分。

**CPU 频率**：`dev.cpu.0.freq`。

### sysctl 名称缓存

`sysctl_names_matching(pred)` — `sysctl -aN` 结果用 `LazyLock<Mutex<Option<Vec<String>>>>` 缓存，仅首次执行。后续按谓词过滤。用于温度发现。

### sysctl 读取

所有 sysctl 读取统一走 `read_sysctl(name)` —— spawn `/sbin/sysctl -n <name>` 子进程，取 stdout trim。**不用** FFI（保持简单，子进程开销在低频采样下可忽略）。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/system/info` | 静态系统信息 |
| GET | `/api/system/metrics` | 实时指标（每次请求采样） |

## 外部依赖

- 系统命令：`/sbin/sysctl`、`/usr/sbin/swapinfo`、`/usr/bin/uptime`
- crate：`parking_lot`、`std::sync::LazyLock`

## 已知限制

- CPU 首次采样返回 0%（无历史数据可做 delta）
- 温度依赖 `coretemp` 模块加载；无传感器的 CPU 返回空数组
- 未采集磁盘 I/O 和网络流量（计划在后续阶段补充）
