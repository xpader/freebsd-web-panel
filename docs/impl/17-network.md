# 17 — 网络接口管理

## 概述

网络接口管理模块提供接口列表、路由表、默认网关的查询，以及基于 rc.conf 的接口配置管理（读/写/应用）、虚拟接口创建与销毁。

所有核心数据通过 FreeBSD 原生 API 获取（`getifaddrs(3)`、`sysctl(NET_RT_DUMP)`、各种 ioctl），不 spawn `ifconfig`/`netstat` 子进程。仅 `sysrc` 用于 rc.conf 读写，`ifconfig` 用于配置应用和接口创建/销毁。

## 数据获取方式

| 数据 | API | 子进程？ |
|---|---|---|
| 接口名/flags/IP地址/MAC/MTU/metric/link_state/baudrate | `getifaddrs(3)` — 遍历 `ifaddrs` 链表 | ❌ |
| 接口分组 | `SIOCGIFGROUP` ioctl（`fill_iface_ioctl`） | ❌ |
| 接口描述 | `SIOCGIFDESCR` ioctl（`fill_iface_ioctl`） | ❌ |
| 驱动状态文本 | `SIOCGIFSTATUS` ioctl（`fill_iface_ioctl`） | ❌ |
| Bridge 成员 | `SIOCGDRVSPEC(BRDGGIFS)` ioctl（`fill_iface_ioctl`） | ❌ |
| 路由表 | `sysctl([CTL_NET, PF_ROUTE, 0, 0, NET_RT_DUMP, 0])` | ❌ |
| rc.conf `defaultrouter` / `ifconfig_*` / `cloned_interfaces`（读） | `sysrc::read_rcconf_files`（直接文件解析） | ❌ |
| rc.conf 写入（网关/接口配置/cloned_interfaces） | `sysrc` 命令 | ✅ |
| DNS 配置 | 直接读写 `/etc/resolv.conf` | ❌ |
| 配置应用/接口创建/销毁 | `ifconfig` 命令 | ✅ |

### getifaddrs 解析

`getifaddrs` 返回一个 `struct ifaddrs` 链表。同一接口有多条记录（每个地址族一条）。
代码用 `BTreeMap<String, NetworkInterface>` 按接口名聚合：

- **AF_INET**: IPv4 地址 (`sockaddr_in`)，含 netmask (`ifa_netmask`) 和 broadcast (`ifa_dstaddr`)
- **AF_INET6**: IPv6 地址 (`sockaddr_in6`)，含 prefix_len（从 netmask 计算）
- **AF_LINK**: `sockaddr_dl` 提供 MAC 地址（`sdl_data[sdl_nlen..]`）和 `struct if_data`（MTU/metric/link_state/baudrate）

flags 从任意记录的 `ifa_flags` 读取（同一接口所有记录的 flags 相同）。

### fill_iface_ioctl — 4 合 1 ioctl 填充

单个 socket fd 上依次执行 4 个 ioctl，填充每接口的额外信息：

1. **SIOCGIFGROUP** → `iface.groups`（两次调用模式：先取长度，再取数据）
2. **SIOCGIFDESCR** → `iface.description`（256 字节缓冲区）
3. **SIOCGIFSTATUS** → `iface.status`（801 字节缓冲区，清理 tab/空白）
4. **SIOCGDRVSPEC(BRDGGIFS)** → `iface.members`（256 条目缓冲区，格式化 info 字符串）

需手动定义的常量（libc crate 未提供）：
- `SIOCGIFGROUP = 0xc0286988`
- `SIOCGIFDESCR = 0xc020692a`
- `SIOCGIFSTATUS = 0xc331693b`
- `SIOCGDRVSPEC = 0xc028697b`
- `BRDGGIFS = 6`

### 路由表解析

通过 `libc::sysctl` 获取 `NET_RT_DUMP` 二进制缓冲区，按 `rtm_msglen` 遍历每条消息。

**关键发现（FreeBSD 15）**：部分路由设置了 `RTA_NETMASK` 位但缓冲区中不包含 netmask sockaddr（占 0 字节）。
遇到 `sa_len == 0` 的 NETMASK 槽位时跳过不前进。

需自定义的结构体和常量：
- `RtMsghdr`（152 字节）+ `RtMetrics`（112 字节，`_filler: [u64; 2]`）
- `RTM_GET`、`RTA_DST/GATEWAY/NETMASK/IFP/IFA` 等常量
- `RTF_UP/GATEWAY/HOST/STATIC/BLACKHOLE` 等常量

## API

### 只读查询

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/network/interfaces` | 全部接口列表（含 groups/description/status/members） |
| GET | `/api/network/interfaces/{name}` | 单接口 rc.conf 配置（返回 `IfaceRcConfConfig`） |
| GET | `/api/network/routes` | 完整路由表（IPv4 + IPv6） |
| GET | `/api/network/gateway` | 默认网关 IPv4+IPv6（运行时值 + rc.conf 持久值） |
| PUT | `/api/network/gateway` | 设置/清除默认网关（IPv4 + IPv6 独立控制，写 rc.conf + 应用路由） |
| GET | `/api/network/dns` | DNS 配置（解析 `/etc/resolv.conf`） |

### 接口配置管理

| 方法 | 路径 | 说明 |
|---|---|---|
| PUT | `/api/network/interfaces/{name}` | 保存配置：先 ifconfig 应用，成功后写 rc.conf |
| POST | `/api/network/interfaces/{name}/apply` | 手动重新应用 rc.conf 配置到运行时 |

### 接口生命周期

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/api/network/interfaces` | 创建虚拟接口（`ifconfig <name> create` + `cloned_interfaces`） |
| DELETE | `/api/network/interfaces/{name}` | 销毁虚拟接口（`ifconfig <name> destroy` + 清理 rc.conf） |

### DNS 管理

| 方法 | 路径 | 说明 |
|---|---|---|
| PUT | `/api/network/dns/nameservers` | 设置 nameservers（原子写 + 备份） |

全部需要认证。

## 数据结构

### 运行时接口信息

```rust
NetworkInterface {
    name, description: Option<String>, status: Option<String>,
    flags: Vec<String>, is_up, is_loopback, is_physical,
    mtu: u32, metric: u32, mac: Option<String>,
    link_state: String, baudrate: u64,
    groups: Vec<String>,           // SIOCGIFGROUP
    members: Vec<BridgeMember>,    // SIOCGDRVSPEC(BRDGGIFS)
    ipv4: Vec<IpConfig>, ipv6: Vec<IpConfig>,
}
BridgeMember { name: String, info: String }
IpConfig { address, netmask, prefix_len, broadcast, is_alias }
Route { destination, gateway, flags, interface, expire }
DefaultGateway {
    gateway: Option, interface: Option, configured: Option,         // IPv4
    gateway6: Option, interface6: Option, configured6: Option,      // IPv6
}
```

### rc.conf 解析配置

```rust
IfaceRcConfConfig {
    interface: String, is_bridge: bool, is_lagg: bool, is_up: bool,
    ipv4: Option<String>, ipv4_netmask: Option<String>,
    ipv4_aliases: Vec<RcIpv4Alias>,     // { address, netmask }
    ipv6_mode: String,                  // "none" | "static" | "slaac"
    ipv6: Vec<RcIpv6Entry>,             // { address, prefixlen }（slaac/none 模式下为空）
    bridge_members: Vec<String>,
    lagg_proto: Option<String>, lagg_ports: Vec<String>,
    mtu: Option<u32>, description: Option<String>,
    options: String,                    // 扩展参数（media/mediaopt/vlan 等不在表单中的 ifconfig 参数）
}
```

### rc.conf 解析与写入

**接口改名处理**：FreeBSD 支持通过 `ifconfig_bridge3="name vvswitch"` 改名。改名后内核 driver name（`bridge3`）不变，可通过 `ifutil::get_drivername()`（sysctl `IFDATA_DRIVERNAME`）获取。rc.conf 中的配置可能存在两种写法：
- **单键**：全部配置在 driver name 键下：`ifconfig_bridge3="inet ... name vvswitch up"`
- **分键**：改名在 driver 键、配置在 live name 键：`ifconfig_bridge3="name vvswitch"` + `ifconfig_vvswitch="inet ... up"`

**解析**（`parse_merged_rcconf(live_name)`）：
1. 用 `resolve_driver_name(live_name)` 获取 driver name
2. 调用 `sysrc::read_rcconf_files()` 直接读取 rc.conf 文件（无子进程，<1ms）
3. 从中解析 driver name 的三个键（primary/aliases/ipv6）
4. 若 driver ≠ live name，额外解析 live name 的三个键，合并字段（live name 的配置优先）
4. 确保 `name <live>` 指令始终存在于 options 中

**IPv4 模式检测**：`ipv4` 为 `null` → `ipv4_mode = "none"`（不配置 IPv4）；主值包含 `DHCP` 或 `SYNCDHCP` → `ipv4_mode = "dhcp"`，前端隐藏 IP/掩码输入；否则为 `static`。

**IPv6 模式检测**：`ifconfig_<name>_ipv6` 键不存在 → `ipv6_mode = "none"`（不配置 IPv6）；值含 `accept_rtadv` → `ipv6_mode = "slaac"`（无静态地址条目），前端隐藏静态 IPv6 输入；否则为 `static`。

**写入**（`build_primary_value` / `build_ipv6_value`）：
- IPv4 none → 主值不含 inet 字段
- IPv4 DHCP → `ifconfig_<driver>="DHCP"`（或 `SYNCDHCP`）
- IPv6 none → `ifconfig_<driver>_ipv6` 键被删除
- IPv6 SLAAC → `ifconfig_<driver>_ipv6="inet6 accept_rtadv"`
- IPv6 static → `build_ipv6_value("static", entries)` 拼接 `inet6 <addr> prefixlen <pl>` 条目
- description 用单引号包裹（`description 'Hello World'`）以区分空格分隔的其他参数。
- `options` 字段中的扩展参数原样追加到主值末尾（如 `media 1000baseTX mediaopt full-duplex`，改名接口含 `name vvswitch`）
- **合并写入**：所有配置统一写入 driver name 键下，并删除 live name 的三个键，消除分键配置

**配置应用**（`apply_ifconfig`）：
1. 先用 `read_interfaces()` 读取当前运行时状态
2. 应用非结构性属性（IP/MTU/description/options/UP），使用 live name 调用 `ifconfig`
3. 应用 LAGG 协议和端口（跳过已有端口）
4. 应用 bridge 成员（跳过已有成员、其他 bridge 的成员）
5. 应用 IPv4 别名（跳过已有地址）
6. 应用 IPv6 条目（跳过已有地址）——仅 `static` 模式应用；`slaac` 和 `none` 模式跳过

**PUT 流程**：先 ifconfig 应用（live name）→ 成功后写 rc.conf（driver name 键）→ 删除 live name 键（合并）。ifconfig 失败则不写 rc.conf，返回错误。

### 默认网关设置

`set_default_gateway()` → `PUT /api/network/gateway`

请求体 `SetGatewayBody { gateway: Option<String>, gateway6: Option<String> }`。两个 family 独立处理，仅提供的字段被更新：

| 场景 | rc.conf 操作 | 路由操作 |
|---|---|---|
| 设置 IPv4 | `sysrc defaultrouter=<gw>` | `route change default <gw>`（失败则 `route add`） |
| 清除 IPv4 | `sysrc -x defaultrouter` | `route delete default` |
| 设置 IPv6 | `sysrc ipv6_defaultrouter=<gw>` | `route -6 change default <gw>`（失败则 `route -6 add`） |
| 清除 IPv6 | `sysrc -x ipv6_defaultrouter` | `route -6 delete default` |

设置前用 `validate_ip` 校验 IP 地址格式。

## 配置按钮可见性规则

以下接口不显示"配置"按钮：
- 有 driver status 文本的接口（如 tap/tun 的 "Opened by PID"、fwe 的 "ch N dma N"）
- 接口名不匹配 `^[a-zA-Z0-9_.]{1,15}$` 的接口

以下接口额外显示"销毁"按钮（红色）：
- 非物理、非 loopback 的虚拟接口（需同时满足配置按钮的条件）

## cloned_interfaces 管理

- **创建接口**：自动将接口名添加到 `cloned_interfaces`（epair 用基名，如 `epair0a` → `epair0`）
- **销毁接口**：从 `cloned_interfaces` 移除对应条目

## Bridge / LAGG 成员选择

前端配置弹窗中，bridge 成员和 LAGG 端口使用下拉选择而非自由输入。候选列表自动过滤：
- 排除接口自身、loopback、其他 bridge/lagg 接口
- bridge 成员排除已在其他 bridge 中的接口（当前 bridge 的成员保留为可选项）

## 前端

- **页面**：`frontend/src/pages/NetworkPage.vue`（Vue 3 SFC）
- **API 客户端**：`frontend/src/lib/api.js`
- **i18n**：`frontend/src/i18n/translations.js` 中 `net.*` 命名空间

### 前端布局

1. 工具栏：创建接口按钮 + 刷新按钮
2. 物理接口卡片网格（含详情/配置按钮）
3. 网桥接口卡片网格（含详情/配置/销毁按钮）
4. 虚拟/其他接口卡片网格（同上）
5. 默认网关卡片（IPv4 + IPv6 运行时值，配置按钮打开网关设置弹窗）
6. 路由表（IPv4/IPv6 分段）

### 网关设置弹窗

通过 `useFormModal` 实现，两个字段：IPv4 网关 + IPv6 网关。预填 rc.conf 中 `defaultrouter` / `ipv6_defaultrouter` 的值。仅修改的字段会包含在 PUT 请求中。

### 配置弹窗

- 接口属性：描述、MTU、扩展选项（Options，自由填写 media/mediaopt/vlan 等额外 ifconfig 参数）
- UP 勾选框
- IPv4 模式切换：无 / DHCP / Static 三选一药丸选择器（`.radio-pill-group`）；无 → 不配置 IPv4，DHCP → 使用 DHCP 获取地址，Static → 显示 IP + 子网掩码输入
- IPv4 别名列表（可增删，`.form-table` 表格布局，无数据时不显示表头）
- IPv6 模式切换：无 / SLAAC / Static 三选一药丸选择器；无 → 不配置 IPv6，SLAAC → 使用路由器广播自动配置，Static → 显示地址 + 前缀长度列表（可增删）
- LAGG 配置（仅 lagg 接口）：协议下拉 + 端口下拉列表
- Bridge 成员（仅 bridge 接口）：成员下拉列表
- 保存按钮：先应用后持久化

### 创建接口弹窗

- 类型选择：Bridge / LAGG / VLAN / TAP / Epair / 自定义
- 编号输入（自定义类型时为名称输入）
- 名称预览（epair 显示 `epair0a epair0b`）

## 文件清单

| 文件 | 说明 |
|---|---|
| `src/handlers/network.rs` | 全部 handler + getifaddrs/sysctl/ioctl 解析 + rc.conf 解析/写入/应用 |
| `src/app.rs` | 路由注册（12 条 network 路由） |
| `frontend/src/pages/NetworkPage.vue` | 接口卡片 + 路由表 + 网关 + 详情弹窗 + 配置弹窗 + 创建弹窗 |
| `frontend/src/i18n/translations.js` | `net.*` 命名空间（en + zh） |
| `frontend/src/assets/app.css` | `.net-iface`、`.config-section`、`.config-grid`、`.checkbox-row`、`.radio-pill-group`、`.form-table` 等样式 |

## 已知限制

- DHCP/SYNCDHCP 配置在 ifconfig apply 时跳过（由 dhclient 管理）
- IPv6 SLAAC/none 配置在 ifconfig apply 时跳过（仅 static 模式应用 IPv6 地址）
- 删除别名/成员需手动销毁后重建（ifconfig 无原子"替换"语义）
- 部分边缘路由的 gateway 显示为空（IPv6 零长度网关地址）
