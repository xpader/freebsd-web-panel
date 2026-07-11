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
| `read_net_counters()` | `getifaddrs(3)` 取 `AF_LINK` 的 `if_data` | `HashMap<String, NetCounters>` | 各接口累计收发字节/包（排除噪音伪接口） |
| `read_net_info()` | `getifaddrs(3)` 聚合 AF_LINK/AF_INET/AF_INET6 | `Vec<NetIfaceInfo>` | 各接口状态/MAC/MTU/IPv4/IPv6/介质，按活跃度降序排序 |

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

网络流量与接口元数据走 `getifaddrs(3)` 系统调用（与 `netstat`/`ifconfig` 内部相同），避免每轮采样 fork/exec 子进程。

- **`read_net_counters()`**：单次遍历 `getifaddrs` 链表，从每个 `AF_LINK` 项的 `ifa_data`（指向 `struct if_data`）读取累计 `ifi_ibytes`/`ifi_obytes`/`ifi_ipackets`/`ifi_opackets`，按接口名聚合为 `HashMap<接口名, NetCounters>`。
- **`read_net_info()`**：同样单次遍历 `getifaddrs`，按接口名聚合多个地址族：`AF_LINK` 取 MTU/UP/RUNNING 标志与 MAC（从 `sockaddr_dl` 解析），`AF_INET` 取 IPv4，`AF_INET6` 取 **全局单播** IPv6（`2000::/3`，跳过 `::1`/`fe80::/10`/`ff00::/8` 等链路本地与组播）。返回 `Vec<NetIfaceInfo{name, mtu, mac, up, running, status, media, ipv4, ipv6}>`。

#### 过滤：最小噪音黑名单

旧实现通过 `is_physical_iface()` 大黑名单（排除 epair、bridge、tap、tun、vm-*、wg、tailscale 等）仅保留"物理网卡"——但这在面板**运行在 Jail 内**（主网卡是 `epair*b`）或**某些虚拟化场景**下会显示"无网络接口"。

新实现把过滤职责拆成两个辅助函数：

- `is_noise_iface()`：只排除**任何场景下都不会承载用户流量**的伪设备：`lo`（loopback）、`pflog`、`pfsync`、`ipfw`、`enc`、`disc`、`edsc`。`read_net_info()` 与 `read_net_counters()` 都用它过滤。
- `is_hardware_iface()`：基于常见 FreeBSD 网卡驱动前缀（bge/em/igb/ix/ixl/ice/mlx/re/vtnet/vmx/hn/axge/cdce/ue/wlan/lagg/vlan/carp）的 allowlist。**仅**供 `handlers::network` 标记 `NetworkInterface.is_physical` 字段使用，不参与仪表盘/监控的数据过滤。

epair、bridge、tap、tun、wg、tailscale、vale、ng、vm-bhyve 桥、gif、gre、stf、faith 等**不再过滤**——它们在某些场景下是主网络出口，直接交给下面的排序权重处理。

#### 排序：按活跃度置顶

`read_net_info()` 返回前按 `iface_rank()` 降序排序（相同权重按名字字典序稳定）：

| 条件 | 加分 |
|---|---|
| `IFF_RUNNING` | +4 |
| `IFF_UP`（但非 RUNNING） | +2 |
| 有 IPv4 地址 | +2 |
| 有全局 IPv6 地址 | +1 |

`handlers::system::collect_network()` 用同样的权重函数（`net_iface_rank`）对仪表盘快照重新排序——这样无论是宿主硬件网卡 `bge0`、Jail 内的 `epair0b`、还是 bhyve 的 `vtnet0`，只要是当前**真正活跃**的口，就会自动出现在仪表盘顶部。仪表盘快照与监控采集都会额外 **`retain` 过滤掉 `!UP` 或没有任何 IP 地址的接口**：bridge、tap、lagg 成员、jail 宿主侧 epair*a 等 UP 但无 IP，以及 DHCP 续约失败/手动 `ifconfig down` 后残留 IP 但已 DOWN 的口都排除；这些口留在"网络"整页看即可。

实时速率（bytes/sec）由调用方基于累计计数器做两次采样差值计算，sysinfo 仅提供瞬时计数器快照（见 [04-system-metrics.md](04-system-metrics.md) 与 [05-monitoring.md](05-monitoring.md)）。

## 外部依赖

- crate：`sysctl`（0.7，sysctl(3) 安全封装）、`libc`（getloadavg、sysctlbyname、getifaddrs/freeifaddrs）

## 测试

`src/sysinfo.rs` 内嵌单元测试（`cargo test sysinfo`）验证真实内核值：
- 字符串/u64 读取非空
- `cp_times` 长度为 5 的倍数
- boot_time 是过去的有效时间戳
- loadavg 合理范围
- 温度读取不 panic
- 网络计数器/接口信息均不含噪音接口（`is_noise_iface` 过滤）
- `read_net_info()` 返回结果满足"rank 降序、同 rank 名字字典序"
- `is_noise_iface` 与 `is_hardware_iface` 对典型样本无交集
- `format_ipv6` 输出规范冒号分隔格式
- `iface_rank` 对 UP+IPv4+IPv6 > UP+IPv4 > UP > DOWN 的排序符合预期
- 活跃接口的 RX 字节总和 > 0（验证 `if_data` 字段读取正确）

## 已知限制

- `read_cp_times` 假设 `long` 为 8 字节（仅 amd64；若未来支持 arm64/i386 需按 `std::mem::size_of::<libc::c_long>()` 动化）
- swap 仍走 `/usr/sbin/swapinfo` 子进程（无对应 sysctl 节点；可用 `kvm_getswapinfo` 但需链 `-lkvm`，暂不引入）
- `format_ipv6` 输出完整 8 组冒号分隔格式（如 `2001:db8:0:0:0:0:0:1`），未实现 RFC 5952 的零组压缩（`2001:db8::1`）；前端目前直接显示原始格式，若需要压缩可在 Rust 或前端各加一个格式化函数
