# 17 — 网络接口管理（只读）

## 概述

网络接口管理模块提供接口列表、路由表和默认网关的只读查询。
P1 阶段实现，所有数据通过 FreeBSD 原生 API 获取（`getifaddrs(3)`、`sysctl(NET_RT_DUMP)`），不 spawn `ifconfig`/`netstat` 子进程。

## 数据获取方式

| 数据 | API | 子进程？ |
|---|---|---|
| 接口名/flags/IP地址/MAC/MTU/metric/link_state | `getifaddrs(3)` — 遍历 `ifaddrs` 链表 | ❌ |
| 路由表 | `sysctl([CTL_NET, PF_ROUTE, 0, 0, NET_RT_DUMP, 0])` | ❌ |
| rc.conf 中的 `defaultrouter` | `sysrc -n defaultrouter` | ✅（唯一子进程） |

### getifaddrs 解析

`getifaddrs` 返回一个 `struct ifaddrs` 链表。同一接口有多条记录（每个地址族一条）。
代码用 `BTreeMap<String, NetworkInterface>` 按接口名聚合：

- **AF_INET**: IPv4 地址 (`sockaddr_in`)，含 netmask (`ifa_netmask`) 和 broadcast (`ifa_dstaddr`)
- **AF_INET6**: IPv6 地址 (`sockaddr_in6`)，含 prefix_len（从 netmask 计算）
- **AF_LINK**: `sockaddr_dl` 提供 MAC 地址（`sdl_data[sdl_nlen..]`）和 `struct if_data`（MTU/metric/link_state）

flags 从任意记录的 `ifa_flags` 读取（同一接口所有记录的 flags 相同）。

### 路由表解析

通过 `libc::sysctl` 获取 `NET_RT_DUMP` 二进制缓冲区，按 `rtm_msglen` 遍历每条消息。

**关键发现（FreeBSD 15）**：部分路由设置了 `RTA_NETMASK` 位但缓冲区中不包含 netmask sockaddr（占 0 字节）。
这导致基于 RTA 位的顺序扫描会将后续 sockaddr 错位读取。

**解决方案**：遇到 `sa_len == 0` 的 NETMASK 槽位时跳过不前进。对于 DST/GATEWAY 的零长度 sockaddr，
按 `sizeof(long)` 前进（这些在 IPv6 路由中确实占据 8 字节）。

需自定义的结构体和常量（libc crate 未提供）：
- `RtMsghdr`（168 字节）+ `RtMetrics`（128 字节）
- `RTM_GET`、`RTA_DST/GATEWAY/NETMASK/IFP/IFA` 等常量
- `RTF_UP/GATEWAY/HOST/STATIC/BLACKHOLE` 等常量

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/network/interfaces` | 全部接口列表 |
| GET | `/api/network/interfaces/{name}` | 单接口详情（404 if not found） |
| GET | `/api/network/routes` | 完整路由表（IPv4 + IPv6） |
| GET | `/api/network/gateway` | 默认网关（运行时值 + rc.conf 持久值） |

全部需要认证。

## 数据结构

```rust
NetworkInterface { name, flags: Vec<String>, is_up, is_loopback, mtu, metric,
                   mac: Option<String>, link_state, ipv4: Vec<IpConfig>, ipv6: Vec<IpConfig> }
IpConfig { address, netmask, prefix_len, broadcast, is_alias }
Route { destination, gateway, flags, interface }
DefaultGateway { gateway: Option, interface: Option, configured: Option }
```

## 文件变更

| 文件 | 动作 |
|---|---|
| `src/handlers/network.rs` | 新建 — 全部 handler + getifaddrs/sysctl 解析逻辑 |
| `src/handlers/mod.rs` | 加 `pub mod network;` |
| `src/app.rs` | 替换 stub 路由为 4 条真实路由 |
| `src/handlers/mod_stubs.rs` | 删除 `status!(network, ...)` |
| `web/js/pages/network.js` | 新建 — 接口卡片 + 路由表 + 网关 + 详情弹窗 |
| `web/js/main.js` | `/network` 从 `makePlannedPage` 改为 `renderNetwork` |
| `web/js/i18n/translations.js` | 新增 `net.*` 命名空间（en + zh） |
| `web/css/app.css` | 新增 `.card-grid`、`.net-iface`、`.kv` 等样式 |

## 已知限制

- **media/description/groups** 未实现（需 SIOCGIFMEDIA/SIOCGIFDESC/SIOCGIFGROUP ioctl，P2）
- **接口修改**（up/down、改 IP、别名增删）未实现（P2）
- **DNS 管理**未实现（P2）
- 部分边缘路由的 gateway 显示为空（IPv6 零长度网关地址）
- 路由表中的 link 层路由显示为 `link:ifname` 而非 netstat 的 `link#N` 格式
