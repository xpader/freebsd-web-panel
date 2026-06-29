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

## 外部依赖

- crate：`sysctl`（0.7，sysctl(3) 安全封装）、`libc`（getloadavg、sysctlbyname）
- 无系统命令依赖

## 测试

`src/sysinfo.rs` 内嵌单元测试（`cargo test sysinfo`）验证真实内核值：
- 字符串/u64 读取非空
- `cp_times` 长度为 5 的倍数
- boot_time 是过去的有效时间戳
- loadavg 合理范围
- 温度读取不 panic

## 已知限制

- `read_cp_times` 假设 `long` 为 8 字节（仅 amd64；若未来支持 arm64/i386 需按 `std::mem::size_of::<libc::c_long>()` 动化）
- swap 仍走 `/usr/sbin/swapinfo` 子进程（无对应 sysctl 节点；可用 `kvm_getswapinfo` 但需链 `-lkvm`，暂不引入）
