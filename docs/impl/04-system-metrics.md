# 04 — 系统指标（实时）

## 概述

仪表盘使用的实时系统指标端点。每次请求即时采样，CPU 使用率通过两次 `kern.cp_times` 采样差值计算。区别于监控模块（周期性持久化历史数据）。

## 实现细节

### 数据获取方式：sysctl(3) 系统调用

所有内核态指标通过 `src/sysinfo.rs` 模块读取，统一走 **sysctl(3) 系统调用**（`sysctl` crate / `libc::sysctlbyname`）——不再 spawn `/sbin/sysctl` 子进程。仅在监控热路径和实时端点共享这些读取器，避免重复实现。

| 读取器 | 实现方式 | 用途 |
|---|---|---|
| `sysinfo::read_string(name)` | `sysctl::Ctl::value_string()` | 字符串型 sysctl（hostname 等） |
| `sysinfo::read_u64(name)` / `read_f64` | `Ctl::value()` 匹配整数变体 | 数值型 sysctl（ncpu、physmem、内存页计数） |
| `sysinfo::read_cp_times()` | `libc::sysctlbyname` 原始缓冲区 | `kern.cp_times`（long 数组，crate 误报为单个 Long） |
| `sysinfo::boot_time()` | `Ctl::value()` → Struct 字节解析 | `kern.boottime`（struct timeval） |
| `sysinfo::read_loadavg()` | `libc::getloadavg()` | 负载均衡（替代解析 `uptime` 输出） |
| `sysinfo::read_core_temps(ncpu)` | `Ctl::value()` → `CtlValue::Temperature` | `dev.cpu.N.temperature`（crate 自动转摄氏） |
| `sysinfo::read_net_counters()` | `netstat -ibn` 解析 | 各接口累计收发字节/包 |
| `sysinfo::read_net_info()` | `ifconfig -a` 解析 | 各接口状态/MAC/MTU/IPv4/介质 |

> 历史原因：`kern.cp_times` 是 `S,LONG` 格式的 long 数组，sysctl crate 会把它当成单个 `Long` 返回，所以该处直接用 `libc::sysctlbyname` 两次调用（先取长度，再取数据）读原始字节再按 8 字节切分 reinterpret。

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

**温度**：`sysinfo::read_core_temps(ncpu)` 遍历 `dev.cpu.0..N.temperature`，sysctl crate 自动将 FreeBSD 的 `IK` 格式转为 `CtlValue::Temperature`，直接取 `.celsius()`。

**CPU 频率**：`dev.cpu.0.freq`（sysinfo::read_u64）。

**网络接口（实时速率 delta）**：`handlers/system.rs::collect_network(now)` 合并两份数据：
1. `sysinfo::read_net_counters()` → 各接口累计字节/包计数器（HashMap）
2. `sysinfo::read_net_info()` → 各接口元数据（up/status/media/IPv4/MAC/MTU）
3. 与 `LAST_NET`（`LazyLock<Mutex<Option<NetSample>>>`）中上次计数器+时间戳做差：`rate = (cur - prev) / (now - prev_ts)`
4. 计数器与元数据按接口名合并，生成 `Vec<NetIface>`（name/rx_bytes/tx_bytes/rx_rate/tx_rate/rx_packets/tx_packets/up/status/media/ipv4/mac/mtu）

关键：`LAST_NET` 是本端点专用静态，与监控模块（`monitor.rs::MONITOR_NET`）独立，避免互相干扰 delta。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/system/info` | 静态系统信息 |
| GET | `/api/system/metrics` | 实时指标（每次请求采样） |

## 外部依赖

- crate：`sysctl`（sysctl(3) 封装）、`libc`（getloadavg、sysctlbyname）、`parking_lot`、`std::sync::LazyLock`
- 系统命令：`/usr/sbin/swapinfo`（swap 统计仍走子进程）、`/usr/bin/netstat`、`/sbin/ifconfig`（网络）
- 详见 [13-sysinfo.md](13-sysinfo.md)

## 前端

`web/js/pages/dashboard.js::renderDashboard` — 仪表盘每 3 秒轮询 `/api/system/metrics` 刷新：静态信息卡片、CPU/内存/Swap/温度指标条，以及网络接口区块。

**网络接口区块**：渲染每个接口一行——名称、链路状态（badge）、IPv4、介质；下方展示实时速率（↓ 下载 / ↑ 上传，`fmtRate` 格式化为 KB/s·MB/s·GB/s）与累计收发字节（`fmtBytes`）。速率由后端 delta 计算，前端直接展示。

## 已知限制

- CPU 首次采样返回 0%（无历史数据可做 delta）
- 温度依赖 `coretemp` 模块加载；无传感器的 CPU 返回空数组
- 网络首次采样速率为 0（无历史计数器可做 delta）
