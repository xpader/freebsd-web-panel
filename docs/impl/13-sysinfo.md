# 13 — sysinfo 共享读取器（sysctl(3) 系统调用）

## 概述

`src/sysinfo.rs` 集中封装所有内核态指标读取，统一通过 **sysctl(3) 系统调用**获取——不再 spawn `/sbin/sysctl` 子进程。供 `handlers/system.rs`（实时端点）和 `monitor.rs`（后台采集器）共用，消除重复实现和 fork/exec 开销。

## 背景

原先每个 sysctl 读取都 `Command::new("/sbin/sysctl").arg("-n")...`，监控热路径每周期 spawn 10+ 进程。改为 sysctl(3) 后：单次内核调用、类型化返回、无字符串解析、无进程创建开销。

## 读取器 API

| 函数 | 实现 | 返回 | 用途 |
|---|---|---|---|
| `read_string(name)` | `sysctl::Ctl::value_string()` | `Option<String>` | 字符串型（hostname、osrelease、hw.model 等） |
| `read_u64(name)` | `Ctl::value()` 匹配所有整数变体 | `Option<u64>` | 数值型（hw.ncpu、hw.physmem、vm.stats.vm.*） |
| `read_f64(name)` | 包装 `read_u64` | `Option<f64>` | 便利封装 |
| `read_cp_times()` | `libc::sysctlbyname` 原始缓冲区 | `Vec<u64>` | `kern.cp_times`（每核 5 个 long） |
| `boot_time()` | `Ctl::value()` → Struct 字节 | `i64`（Unix 秒） | `kern.boottime`（struct timeval） |
| `read_loadavg()` | `libc::getloadavg()` | `[f64; 3]` | 1/5/15 分钟负载 |
| `read_core_temps(ncpu)` | `Ctl::value()` → `Temperature` | `Vec<(usize, f32)>` | 各核摄氏温度 |
| `read_net_counters()` | `netstat -ibn` 解析 `<Link#>` 行 | `HashMap<String, NetCounters>` | 各接口累计收发字节/包（排除 lo*） |
| `read_net_info()` | `ifconfig -a` 逐行解析 | `Vec<NetIfaceInfo>` | 各接口状态/MAC/MTU/IPv4/介质（排除 lo*） |

## 关键实现细节

### `kern.cp_times` 的特殊处理

该 sysctl 格式为 `S,LONG`（long 数组），但 `sysctl` crate 会把它误报为单个 `Long` 值返回。因此 `read_cp_times()` 直接调用 `libc::sysctlbyname`：

1. 第一次调用：`buf=NULL`，获取所需缓冲区长度
2. 第二次调用：分配 buffer，填充数据
3. 按 8 字节（amd64 的 `long`）`from_ne_bytes` 切片 reinterpret 为 `i64`，再转 `u64`

详见 `src/sysinfo.rs::read_long_array`。

### 温度的自动转换

FreeBSD 温度 sysctl 用 `IK` 格式字符串（deciKelvin 等）。`sysctl` crate 检测到该格式后自动返回 `CtlValue::Temperature`，调用 `.celsius()` 即得摄氏度——无需手动解析 `"44.0C"` 字符串。

### boottime 解析

`kern.boottime` 返回 `struct timeval`。在 amd64 上 `tv_sec` 和 `tv_usec` 各 8 字节。读取 `CtlValue::Struct` 的前 8 字节 `from_ne_bytes` 为 `i64` 即得启动 Unix 时间戳。

### 网络接口读取

网络流量与接口元数据无对应 sysctl 节点，仍走子进程解析。仅返回**物理网卡**，虚拟/伪接口通过 `is_physical_iface()` denylist 过滤掉（loopback、jail epair、bridge、tap、tun、VPN 隧道、netgraph、vm-bhyve 桥 `vm-*` 等），避免仪表盘充斥虚拟接口流量。

- **`read_net_counters()`**：`netstat -ibn`（`-b` 显示字节计数，`-n` 跳过反向解析）。只取 `<Link#N>` 行（携带原始字节/包总计）。
  - **列索引从表头解析**（非硬编码），以适配不同 FreeBSD 版本的列差异——例如 FreeBSD 15.x 在 `Ierrs` 与 `Ibytes` 之间多了一列 `Idrop`，若用固定下标会把 Idrop（恒 0）当 Ibytes、把 Oerrs（桥接口上因泛洪可能很大）当 Obytes，导致物理网卡显示零流量、虚拟桥显示假流量。
  - 返回 `HashMap<接口名, NetCounters{rx_bytes, tx_bytes, rx_packets, tx_packets}>`。
- **`read_net_info()`**：`ifconfig -a` 逐行解析。顶格行（首字符非空白）为接口定义（提取 name、`<UP...>` 判定 up、`mtu N`），缩进行为属性（`inet`/`ether`/`media:`/`status:`）。同样用 `is_physical_iface()` 过滤。返回 `Vec<NetIfaceInfo{name, mtu, mac, up, status, media, ipv4}>`。

实时速率（bytes/sec）由调用方基于累计计数器做两次采样差值计算，sysinfo 仅提供瞬时计数器快照（见 [04-system-metrics.md](04-system-metrics.md) 与 [05-monitoring.md](05-monitoring.md)）。

## 外部依赖

- crate：`sysctl`（0.7，sysctl(3) 安全封装）、`libc`（getloadavg、sysctlbyname）
- 系统命令：`/usr/bin/netstat`（网络流量计数器）、`/sbin/ifconfig`（网络接口元数据）

## 测试

`src/sysinfo.rs` 内嵌单元测试（`cargo test sysinfo`）验证真实内核值：
- 字符串/u64 读取非空
- `cp_times` 长度为 5 的倍数
- boot_time 是过去的有效时间戳
- loadavg 合理范围
- 温度读取不 panic
- 网络计数器/接口信息均仅含物理网卡（`is_physical_iface` 过滤）
- 活跃物理网卡的 RX 字节总和 > 0（验证列解析正确，未误读 Idrop）

## 已知限制

- `read_cp_times` 假设 `long` 为 8 字节（仅 amd64；若未来支持 arm64/i386 需按 `std::mem::size_of::<libc::c_long>()` 动化）
- swap 仍走 `/usr/sbin/swapinfo` 子进程（无对应 sysctl 节点；可用 `kvm_getswapinfo` 但需链 `-lkvm`，暂不引入）
- 网络数据走子进程（无对应 sysctl 节点）；`netstat`/`ifconfig` 输出格式解析依赖表头列名（`netstat`）和行首缩进/关键字（`ifconfig`），若未来 FreeBSD 调整输出格式需同步维护
