# 模块设计：网络接口管理

> 原 `40-system.md §3` 的简要网络规划，本文档为展开的详细设计方案。

## 1. 目标

在 Web 面板中管理 FreeBSD 网络接口：查看接口状态与配置、修改 IP/MTU/描述、管理 IP 别名、查看路由表、修改默认网关与 DNS。

分两期实现：

| 期 | 范围 | 状态 |
|---|---|---|
| **P1 只读** | 接口列表/详情、路由表、默认网关查看 | 本次实现 |
| **P2 读写** | 改 IP、up/down、别名增删、MTU、描述、默认网关、DNS、rc.conf 持久化 | 后续 |

---

## 2. 数据模型

```rust
/// 一个网络接口的完整信息。
/// 数据来源：getifaddrs(3) — AF_INET/AF_INET6/AF_LINK 条目聚合
struct NetworkInterface {
    name: String,                 // ifa_name
    flags: Vec<String>,           // ifa_flags → IFF_* 位解码
    is_up: bool,                  // ifa_flags & IFF_UP
    is_loopback: bool,            // ifa_flags & IFF_LOOPBACK
    mtu: u32,                     // if_data.ifi_mtu (来自 AF_LINK 条目)
    metric: u32,                  // if_data.ifi_metric
    mac: Option<String>,          // sockaddr_dl.sdl_data (来自 AF_LINK 条目)
    link_state: String,           // if_data.ifi_link_state → "up"/"down"/"unknown"
    media: Option<String>,        // SIOCGIFMEDIA ioctl（P2 可选增强）
    description: Option<String>,  // SIOCGIFDESC ioctl（P2 可选增强）
    ipv4: Vec<IpConfig>,          // AF_INET 条目
    ipv6: Vec<IpConfig>,          // AF_INET6 条目
    groups: Vec<String>,          // SIOCGIFGROUP ioctl（P2 可选增强）
}

struct IpConfig {
    address: String,              // "192.168.1.100" 或 "fe80::1"
    netmask: Option<String>,      // 来自 ifa_netmask（点分十进制，非十六进制）
    prefix_len: Option<u8>,       // v6 prefix len, 从 netmask 计算
    broadcast: Option<String>,    // v4 广播地址（来自 ifa_broadaddr）
    is_alias: bool,               // 多个 IPv4 时第一个为主，其余为别名
}

/// 路由表条目。数据来源：sysctl(NET_RT_DUMP)
struct Route {
    destination: String,          // 来自 RTA_DST sockaddr
    gateway: String,              // 来自 RTA_GATEWAY sockaddr
    flags: String,                // rtm_flags → RTF_* 位解码
    interface: String,            // 来自 RTA_IFP (sockaddr_dl) 或 rtm_index → if_indextoname
}

/// 默认网关信息。
struct DefaultGateway {
    gateway: Option<String>,      // 路由表中 destination="default" 的 gateway
    interface: Option<String>,    // 关联接口
    configured: Option<String>,   // sysrc -n defaultrouter 的值
}
```

### IP 地址用 String 而非 IpAddr 的原因

`getifaddrs` 返回的是 `sockaddr_in`/`sockaddr_in6` 结构体，我们将其格式化为字符串。
netmask 同样从 `sockaddr_in.sin_addr` 格式化为点分十进制（如 `255.255.255.0`），比 ifconfig 的十六进制格式（`0xffffff00`）更直观。

---

## 3. 后端实现

### 3.0 数据获取方式：原生 API vs 命令行

**结论：核心数据全部走 FreeBSD 原生 API（libc crate），不 spawn 子进程。**

| 数据 | 方法 | libc crate 支持？ | 需要命令？ |
|---|---|---|---|
| 接口名、flags、IPv4/IPv6 地址、netmask、broadcast、MAC | **`getifaddrs(3)`** | ✅ 已有 `libc::getifaddrs`、`libc::ifaddrs` 结构体 | ❌ |
| MTU、metric、link_state、流量计数器 | **`getifaddrs` 的 `ifa_data` → `struct if_data`** | ✅ 已有 `libc::if_data`（含 `ifi_mtu`/`ifi_metric`/`ifi_link_state`） | ❌ |
| IFF_UP/BROADCAST/LOOPBACK/RUNNING/SIMPLEX/MULTICAST flags | **`ifa_flags` 位掩码** | ✅ 已有所有 `libc::IFF_*` 常量 | ❌ |
| MAC 地址（AF_LINK） | **`sockaddr_dl`** 的 `sdl_data` | ✅ 已有 `libc::sockaddr_dl` | ❌ |
| 路由表 | **`sysctl(NET_RT_DUMP)`** | ⚠️ 有 `NET_RT_DUMP` 常量和 `sysctl` 函数，但 `rt_msghdr` 结构体和 `RTA_*` 常量需自己定义（约 30 行） | ❌ |
| media 类型字符串（"1000baseT FD"） | `SIOCGIFMEDIA` ioctl | ❌ `SIOCGIFMEDIA` 和 `struct ifmediareq` 需自己定义 | ⚠️ 可选 |
| 接口描述（description） | `SIOCGIFDESC` ioctl | ❌ 需自己定义常量 | ⚠️ 可选 |
| 接口分组（groups） | `SIOCGIFGROUP` ioctl | ❌ 需自己定义常量 | ⚠️ 可选 |

**策略**：
1. **P1 核心字段**（name/flags/IP/MAC/MTU/metric/link_state）：全部用 `getifaddrs`，零子进程。
2. **路由表**：用 `sysctl(NET_RT_DUMP)`，零子进程。需自己定义 `rt_msghdr` 结构体和 ~10 个常量。
3. **media/description/groups**（可选增强）：用 ioctl（需自己定义常量和结构体，~50 行代码）。**初始版本可先省略**，前端显示"—"。

这比 ifconfig 文本解析更优：**结构化数据、无格式依赖、无子进程开销、更安全**。

### 3.1 接口信息：getifaddrs 解析

`getifaddrs(3)` 返回一个 `struct ifaddrs` 链表。**同一接口会有多条记录**——每个地址族一条。

```
链表遍历逻辑：
  for each ifaddrs entry:
    name = ifa_name                          // "bge0"
    flags = ifa_flags                        // IFF_UP | IFF_BROADCAST | ...
    family = ifa_addr.sa_family

    AF_INET  → IPv4 地址 (sockaddr_in)
               + netmask (ifa_netmask)
               + broadcast (ifa_broadaddr)

    AF_INET6 → IPv6 地址 (sockaddr_in6)
               + netmask (ifa_netmask)

    AF_LINK  → sockaddr_dl
               sdl_data[0..sdl_nlen] = 接口名
               sdl_data[sdl_nlen..sdl_nlen+sdl_alen] = MAC 地址
               + ifa_data → struct if_data
                   ifi_mtu = MTU
                   ifi_metric = metric
                   ifi_link_state = 0(unknown)/1(down)/2(up)
                   ifi_type = 接口类型 (IFT_ETHER, IFT_LOOP, ...)
```

**关键逻辑**：
- **接口去重与合并**：同一 `ifa_name` 的 AF_INET/AF_INET6/AF_LINK 记录需合并为一个 `NetworkInterface`。用 `HashMap<String, NetworkInterface>` 聚合。
- **主地址 vs alias**：`getifaddrs` 不直接标记 alias。判断方式：同一接口的多个 IPv4 地址中，第一个非 alias 的为主地址。**更准确的方案**：用 `ifa_flags` 不区分主/alias——FreeBSD 内核不标记别名，alias 是 ifconfig 命令的概念。前端展示时，如果有多个 IPv4 地址，第一个标为主，其余标为别名即可。
- **is_up**：`ifa_flags & IFF_UP != 0`
- **is_loopback**：`ifa_flags & IFF_LOOPBACK != 0`（比判断 name=="lo*" 更准确）
- **link_state**：从 `if_data.ifi_link_state` 读取：`0` = unknown, `1` = down, `2` = up。映射为 status 字符串。
- **MAC 地址**：从 `sockaddr_dl` 的 `sdl_data[sdl_nlen..sdl_nlen+sdl_alen]` 提取 6 字节，格式化为 `xx:xx:xx:xx:xx:xx`。
- **netmask 格式**：`sockaddr_in` 中是 `sin_addr.s_addr`（网络字节序的 IPv4），直接格式化为点分十进制。不需要十六进制 `0xffffff00` 格式。

### 3.2 路由表：sysctl(NET_RT_DUMP)

通过 sysctl MIB `[CTL_NET, PF_ROUTE, 0, AF, NET_RT_DUMP, 0]` 获取完整路由表的二进制缓冲区。

```rust
// 需自己定义的常量和结构体（libc crate 未提供）：
const RTM_GET: c_int = 0x4;
const RTA_DST: c_int = 0x1;
const RTA_GATEWAY: c_int = 0x2;
const RTA_NETMASK: c_int = 0x4;
const RTA_IFP: c_int = 0x10;
const RTA_IFA: c_int = 0x20;

const RTF_UP: c_int = 0x1;
const RTF_GATEWAY: c_int = 0x2;
const RTF_HOST: c_int = 0x4;
const RTF_STATIC: c_int = 0x800;
const RTF_BLACKHOLE: c_int = 0x1000;

#[repr(C)]
struct rt_msghdr {
    rtm_msglen: u16,
    rtm_version: u8,
    rtm_type: u8,
    rtm_index: u16,
    _rtm_spare1: i16,
    rtm_flags: i32,
    rtm_addrs: i32,
    rtm_pid: i32,
    rtm_seq: i32,
    rtm_errno: i32,
    rtm_fmask: i32,
    rtm_inits: u32,
    rtm_rmx: [u8; 56],  // rt_metrics，不展开，我们不解析 metrics
}
```

**解析流程**：
1. `sysctl([CTL_NET, PF_ROUTE, 0, AF_UNSPEC, NET_RT_DUMP, 0])` 获取缓冲区
2. 按 `rtm_msglen` 遍历每条消息
3. 每条消息后跟变长 sockaddr 数组，按 `rtm_addrs` 位掩码顺序排列：
   - 依次检查 `RTA_DST` → `RTA_GATEWAY` → `RTA_NETMASK` → `RTA_IFP` → `RTA_IFA`
   - 每个 sockaddr 按 `sa_len` 向上取整（8 字节对齐）跳过
4. 提取 destination、gateway、interface 信息

### 3.3 默认网关

- 运行时值：从路由表中 destination == "default" 的条目提取
- 持久化值：`sysrc -n defaultrouter`（需 spawn sysrc，但只有一个简短命令）
- 二者都返回给前端，前端可提示"配置与运行时不一致"

### 3.4 模块结构

```
src/handlers/network.rs    # handler 函数 + 数据结构
```

辅助函数（模块私有）：
- `fn read_interfaces() -> ApiResult<Vec<NetworkInterface>>` — 调用 `getifaddrs`，遍历链表聚合
- `fn read_routes() -> ApiResult<Vec<Route>>` — 调用 `sysctl(NET_RT_DUMP)`，解析二进制消息
- `fn flags_to_strings(flags: u32) -> Vec<String>` — IFF_* 位解码
- `fn route_flags_to_strings(flags: i32) -> String` — RTF_* 位解码
- `fn sockaddr_to_string(sa: *const sockaddr) -> Option<String>` — 通用 sockaddr → IP 字符串
- `fn extract_mac(sdl: &sockaddr_dl) -> Option<String>` — 从 sockaddr_dl 提取 MAC

### 3.5 输入校验

接口名校验：`^[a-zA-Z0-9_.]+$`，1–15 字符（与 rcconf 模块的校验风格一致）。

---

## 4. API 设计

### P1（只读，本次实现）

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/api/network/interfaces` | 全部接口列表 | ✅ |
| GET | `/api/network/interfaces/{name}` | 单接口详情（404 if not found） | ✅ |
| GET | `/api/network/routes` | 路由表 | ✅ |
| GET | `/api/network/gateway` | 默认网关（含 rc.conf 持久值） | ✅ |

### P2（读写，后续）

| 方法 | 路径 | 说明 |
|---|---|---|
| PUT | `/api/network/interfaces/{name}` | 修改接口属性（MTU、描述、IP、netmask），body 含 `persist: bool` |
| POST | `/api/network/interfaces/{name}/up` | 启用接口 |
| POST | `/api/network/interfaces/{name}/down` | 停用接口 |
| POST | `/api/network/interfaces/{name}/aliases` | 添加 IP 别名 |
| DELETE | `/api/network/interfaces/{name}/aliases/{ip}` | 删除 IP 别名 |
| PUT | `/api/network/gateway` | 设置默认网关 + 持久化 `defaultrouter` |
| GET | `/api/network/dns` | 读取 `/etc/resolv.conf` |
| PUT | `/api/network/dns` | 写入 `/etc/resolv.conf` |

---

## 5. 前端 UI 设计

### 5.1 页面布局

`/network` 路由替换现有的 planned 占位页，新建 `web/js/pages/network.js`。

页面分三个区域（从上到下）：

```
┌──────────────────────────────────────────────────┐
│  网络接口                          [刷新] 按钮      │  ← page-header + toolbar
├──────────────────────────────────────────────────┤
│  ┌────────────┐  ┌────────────┐  ┌────────────┐ │
│  │   bge0     │  │   lo0      │  │   vmx0     │ │  ← 接口卡片网格
│  │  ● active  │  │  ● running │  │  ○ no car. │ │     每卡：名称、状态灯、
│  │ 10.0.0.5   │  │ 127.0.0.1  │  │   (无 IP)   │ │     IPv4、IPv6、MAC、
│  │ /24        │  │            │  │             │ │     media、MTU
│  │ [详情]      │  │            │  │             │ │
│  └────────────┘  └────────────┘  └────────────┘ │
├──────────────────────────────────────────────────┤
│  默认网关：192.168.1.1 (bge0)                      │  ← 网关信息卡
│  rc.conf 配置：192.168.1.1 ✓                       │
├──────────────────────────────────────────────────┤
│  路由表                                            │
│  ┌──────────────────────────────────────────────┐ │
│  │ Destination   Gateway       Flags  Interface │ │  ← 路由表（折叠/可展开）
│  │ default       192.168.1.1   UGS    bge0      │ │
│  │ 127.0.0.0/8   127.0.0.1     URS    lo0       │ │
│  │ ...                                           │ │
│  └──────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
```

### 5.2 接口卡片设计

每张卡片（grid 布局，响应式 2-3 列）：

```
┌─────────────────────────────┐
│  🌐 bge0          ● active   │  ← 名称 + 状态灯（绿=up+active, 灰=down）
│  ───────────────────────────│
│  IPv4: 192.168.1.100/24     │
│  IPv4: 192.168.1.101/32 ╌╌  │  ← 别名用虚线/淡色区分
│  IPv6: fe80::xxx/64         │
│  MAC:  00:1b:21:aa:bb:cc    │
│  Media: 1000baseT FD        │
│  MTU: 1500                  │
│  ───────────────────────────│
│  [详情]                       │  ← P2: [+ 别名] [▲/▼ 启停] [编辑]
└─────────────────────────────┘
```

### 5.3 接口详情弹窗

点击 [详情] 打开 modal，展示完整信息：

```
┌──────── bge0 详细信息 ────────┐
│                                │
│  状态      ● active (UP)        │
│  Flags    UP,BROADCAST,...     │
│  Metric   0                    │
│  MTU      1500                 │
│  MAC      00:1b:21:aa:bb:cc    │
│  Media    1000baseT <FD>       │
│  描述     Main LAN              │
│  分组     lan                   │
│                                │
│  IPv4 地址                      │
│  ┌──────────────────────────┐  │
│  │ 192.168.1.100/24         │  │
│  │   broadcast 192.168.1.255│  │
│  ├──────────────────────────┤  │
│  │ 192.168.1.101/32 (alias) │  │
│  │   broadcast 192.168.1.101│  │
│  └──────────────────────────┘  │
│                                │
│  IPv6 地址                      │
│  ┌──────────────────────────┐  │
│  │ fe80::xxx/64  scope 0x2  │  │
│  └──────────────────────────┘  │
│                                │
│              [关闭]             │
└────────────────────────────────┘
```

### 5.4 路由表区

简单表格，放在页面下方。列：Destination、Gateway、Flags、Interface。
支持 IPv4 / IPv6 分开显示（两个子表或一个 tab）。

### 5.5 交互细节

- **刷新按钮**：重新拉取所有数据
- **加载状态**：spinner + loading 文案
- **错误处理**：卡片区域显示错误信息（与 rcconf.js 一致）
- **P2 操作按钮**：up/down 用 confirmDialog 二次确认；编辑用 formModal

### 5.6 前端文件清单

| 文件 | 动作 |
|---|---|
| `web/js/pages/network.js` | **新建** — 接口列表+路由+网关 |
| `web/js/main.js` | 将 `/network` 从 `makePlannedPage` 改为 `renderNetwork` |
| `web/js/i18n/translations.js` | 新增 `network.*` 命名空间键 |

---

## 6. i18n 键规划

遵循现有命名规范（同义复用 common，语义不同才新建）：

```js
network: {
  title: 'Network Interfaces',        // 页面标题
  subtitle: 'View and manage network interfaces and routing',
  // 接口属性（复用 common 已有的：status, name, type, device）
  ipv4: 'IPv4 Address',
  ipv6: 'IPv6 Address',
  mac: 'MAC Address',
  media: 'Media',
  mtu: 'MTU',
  metric: 'Metric',
  flags: 'Flags',
  description: 'Description',
  groups: 'Groups',
  alias: 'alias',
  up: 'Up',
  down: 'Down',
  active: 'Active',
  noCarrier: 'No Carrier',
  running: 'Running',
  noInterfaces: 'No network interfaces found',
  detail: 'Detail',
  // 路由表
  routes: 'Routing Table',
  routesV4: 'IPv4 Routes',
  routesV6: 'IPv6 Routes',
  destination: 'Destination',
  gateway: 'Gateway',
  // 默认网关
  defaultGateway: 'Default Gateway',
  gatewayConfigured: 'rc.conf Configured',
  gatewayMismatch: 'Running-time gateway differs from rc.conf',
  notConfigured: 'Not configured',
}
```

需确认与 common 中的同义词：`edit`、`save`、`delete`、`cancel`、`confirm`、`refresh` 等一律复用 common。

---

## 7. 安全考虑

1. **接口名校验**：`^[a-zA-Z0-9_.]+$`，防注入（与项目约定一致）
2. **命令执行**：始终 `Command::new().arg()`，禁止字符串拼接 shell
3. **P2 危险操作**：down 接口可能导致面板自身失联——前端需显示明确警告
4. **IP 地址校验**（P2）：写入时校验 IPv4/IPv6 格式 + netmask 合法性
5. **只读优先**：P1 纯读取，无安全风险，先上线

---

## 8. 持久化策略（P2）

修改接口配置时，通过 `sysrc` 写入 rc.conf 对应键：

| 操作 | rc.conf 键 | 示例 |
|---|---|---|
| 主 IP | `ifconfig_<if>` | `ifconfig_bge0="inet 192.168.1.100 netmask 255.255.255.0"` |
| 别名 | `ifconfig_<if>_alias0` | `ifconfig_bge0_alias0="inet 192.168.1.101 netmask 255.255.255.255"` |
| 描述 | `ifconfig_<if>_description` | `ifconfig_bge0_description="Main LAN"` |
| MTU | 无直接键 | 需通过 `ifconfig_<if>` 值中追加 `mtu N` |
| 默认网关 | `defaultrouter` | `defaultrouter="192.168.1.1"` |

前端切换 `persist` 开关决定是否写 rc.conf。运行时修改用 `ifconfig`，持久化用 `sysrc`。

---

## 9. 实现步骤（P1）

1. 后端 `src/handlers/network.rs`：用 `getifaddrs(3)` 读取接口数据（name/flags/IP/MAC/MTU/metric/link_state）
2. 后端：用 `sysctl(NET_RT_DUMP)` 读取路由表（定义 `rt_msghdr` + `RTA_*` 常量，解析二进制消息）
3. 后端：默认网关查询（路由表 default 条目 + sysrc defaultrouter）
4. 后端：注册路由到 `app.rs`，从 `mod_stubs` 移除 network
5. 前端 `web/js/pages/network.js`：卡片布局 + 路由表 + 网关
6. 前端：注册路由，接入 i18n
7. 文档：写 `docs/impl/17-network.md`
8. 测试：编译 + `node --check` + 手动验证
