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
| GET | `/api/network/static-routes` | 静态路由列表（从 rc.conf 解析） |
| POST | `/api/network/static-routes` | 添加静态路由（写 rc.conf + 应用 `route add`） |
| PUT | `/api/network/static-routes/{name}` | 修改静态路由（更新 rc.conf + 替换运行时路由） |
| DELETE | `/api/network/static-routes/{name}` | 删除静态路由（从 rc.conf 移除 + `route delete`） |

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
    name: Option<String>,               // 改名目标（None/空 → 沿用 driver 名）。来自 ifconfig_<driver>_name 或剥离的 name 指令
}
```

### rc.conf 解析与写入
**接口改名机制**：使用 FreeBSD rc.d 框架原生的 `ifconfig_<driver>_name` 语义，而非内嵌 `name` 指令。原因：rc.d 的 `ifconfig_up`（network.subr:167-172）执行 `eval ifconfig $1 $args` 时，若 `args` 含 `name vvswitch`，接口立即改名，但函数继续用原名 `$1` 做后续操作（`ipv4_up`/`ipv6_up`/`childif_create`/收尾 `ifconfig $1 up`），全部对已不存在的原名落空 → `interface bridge2 does not exist`。

`ifconfig_<driver>_name` 由 rc.d 的 `ifnet_rename()`（network.subr:1591）在 `ifn_start` **之前**、独立的一步执行改名，改完后 `list_net_interfaces` 枚举到的已是新名，后续配置天然地按新名走。

**关键**：改名接口的配置必须写在 **live（改名后）名字键**下，不能写在 driver 键下。因为 `_ifconfig_getargs` → `get_if_var` 只按当前 live 名查 `ifconfig_<live>`，不解析 driver 名；启动改名后 `ifconfig_bridge2` 是死键、永远不会被应用。

| 接口状态 | rc.conf 布局 |
|---|---|
| 未改名（live == driver == `bridge2`） | `ifconfig_bridge2="..."` + `_aliases` + `_ipv6`；无 `_name` |
| 已改名（driver=`bridge2`, live=`vvswitch`） | `ifconfig_bridge2_name="vvswitch"` + `ifconfig_vvswitch="..."` + `_aliases` + `_ipv6` |

改名后内核 driver name（`bridge2`）不变，可通过 `ifutil::get_drivername()`（sysctl `IFDATA_DRIVERNAME`）获取。

**解析**（`parse_merged_rcconf(live_name)`）：
1. 用 `resolve_driver_name(live_name)` 获取 driver name
2. 调用 `sysrc::read_rcconf_files()` 直接读取 rc.conf 文件（无子进程，<1ms）
3. **主配置来自 live name 键**（`parse_iface_rcconf(live_name)`）
4. 兼容旧风格：若 driver ≠ live，额外解析 driver 键（旧内嵌 `name` 布局），仅填充 live 键为空的字段；`is_up` 取两者 OR
5. **改名目标 `cfg.name`** 优先级：显式 `ifconfig_<driver>_name` 键 > 从主值剥离的 `name <X>` 指令 > 旧 driver 键剥离的 `name` > 推断（live 名不是该 driver 的默认名 → 改名）；若 name == driver 则归一化为 None。**epair 特例**：epair 的 driver 名是基名（如 `epair0`），live 名为 `epair0a`/`epair0b`（默认名，二者天然不同）；`is_default_iface_name()` 识别 `epairNa|b` 形式，避免把默认命名的 epair 误判为改名
6. `parse_ifconfig_tokens` 遇到 `name` 时消费下一个 token 作为改名目标，**不**压入 options_tokens

**IPv4 模式检测**：`ipv4` 为 `null` → `ipv4_mode = "none"`（不配置 IPv4）；主值包含 `DHCP` 或 `SYNCDHCP` → `ipv4_mode = "dhcp"`，前端隐藏 IP/掩码输入；否则为 `static`。

**IPv6 模式检测**：`ifconfig_<name>_ipv6` 键不存在 → `ipv6_mode = "none"`（不配置 IPv6）；值含 `accept_rtadv` → `ipv6_mode = "slaac"`（无静态地址条目），前端隐藏静态 IPv6 输入；否则为 `static`。

**写入**（`build_primary_value` / `build_ipv6_value`）：
- IPv4 none → 主值不含 inet 字段
- IPv4 DHCP → `ifconfig_<name>="DHCP"`（或 `SYNCDHCP`）
- IPv6 none → `ifconfig_<name>_ipv6` 键被删除
- IPv6 SLAAC → `ifconfig_<name>_ipv6="inet6 accept_rtadv"`
- IPv6 static → `build_ipv6_value("static", entries)` 拼接 `inet6 <addr> prefixlen <pl>` 条目
- description 用单引号包裹（`description 'Hello World'`）以区分空格分隔的其他参数。
- `options` 字段中的扩展参数原样追加到主值末尾（如 `media 1000baseTX mediaopt full-duplex`）。`name` 指令已从 options 中剥离，由独立的 `ifconfig_<driver>_name` 键承载
- **写入目标键**：配置写入 **live name 键**（`ifconfig_<target>`/`_aliases`/`_ipv6`）。改名时额外写 `ifconfig_<driver>_name=<target>`；撤销改名（target == driver）则删除该键。同时清理旧键（原 live 名键 + 旧 driver 名配置键），仅保留 target 名键

**配置应用**（`apply_ifconfig(name, old, cfg)`，`old` = 修改前 rc.conf 快照）：
1. 先用 `read_interfaces()` 读取当前运行时状态
2. 应用非结构性属性（IP/MTU/description/options/UP），使用 live name 调用 `ifconfig`。纯 `inet <addr>` 会替换当前主地址，因此主 IP/掩码变更在此收敛
3. 应用 LAGG 协议和端口（跳过已有端口）
4. 应用 bridge 成员（跳过已有成员、其他 bridge 的成员）
5. 应用 IPv4 别名；已存在但掩码不符的别名先 `inet <addr> delete` 再重加（`alias` 不会更新掩码）
6. 应用 IPv6 条目（跳过已有地址）——仅 `static` 模式应用；`slaac` 和 `none` 模式跳过
7. **删除协调**（old 配置驱动）：重读 live 状态后，删除「old 配置管理过、新配置不再包含、live 仍存在」的地址与成员：
   - IPv4：old 主 IP + 别名 − 新集合 → `ifconfig <if> inet <addr> delete`（切换到 DHCP 时旧静态主 IP 也会被删除）
   - IPv6：old static 条目 − 新集合 → `inet6 <addr> delete`；永不删除 fe80::/10 链路本地地址
   - bridge 成员 → `deletem`；lagg 端口 → `-laggport`
   - **删除集合从 old rc.conf 推导而非 live 差集**——live 有但配置没有的地址可能是 dhclient 分配或管理员 out-of-band 添加的，不属于面板管理范围，误删可能把用户锁在面板外。已在 live 消失的条目自动跳过（幂等）
   - `managed_v4`/`managed_v6` 定义「配置管理的地址集合」：DHCP 主 IP 排除、slaac 排除、fe80::/10 排除

**PUT 流程**：⓪ 先快照 old 配置（`parse_merged_rcconf`，必须在改名前，函数用 live 名解析键）→ ① 若 target ≠ live name，先 `ifconfig <live> name <target>` 独立改名（先校验目标名未被占用）→ ② `apply_ifconfig(<target>, old, new)` 应用配置 → ③ 写 rc.conf（target 名键 + `ifconfig_<driver>_name`）→ ④ 清理旧键 → ⑤ `restore_default_routes()` 恢复默认路由（见下）→ ⑥ 用 target 名回读合并配置。ifconfig 改名或应用失败则不写 rc.conf，返回错误。改名后原 live 名已不存在，必须用 target 名回读（否则 `resolve_driver_name` 失败）。

**网关自动恢复**（`restore_default_routes`）：内核在删除接口地址时会一并删除引用该地址的路由（重新加回 IP 不会恢复路由），接口配置变更因此可能弄丢默认网关。PUT/apply 成功后，将 rc.conf 的 `defaultrouter`/`ipv6_defaultrouter` 与 live 默认路由比对（`same_gw` 忽略 IPv6 zone 后缀），不一致则 `route change`/`add` 恢复——对应系统启动 netif → routing 的顺序。失败记入审计日志备注，不中断操作。

### 默认网关设置

请求体 `SetGatewayBody { gateway: Option<String>, gateway6: Option<String> }`。两个 family 独立处理，仅提供的字段被更新：

| 场景 | 校验 | 路由操作（先） | rc.conf 操作（后） |
|---|---|---|---|
| 设置 IPv4 | 严格 `Ipv4Addr` | `route change default <gw>`，失败回退 `route add`，仍失败 → 422 返回 route(8) stderr | `sysrc defaultrouter=<gw>` |
| 清除 IPv4 | — | `route delete default`（"not in table"/"has not been found" 视为成功） | `sysrc -x defaultrouter` |
| 设置 IPv6 | `Ipv6Addr` + 可选 `%zone` 后缀 | `route -6 change default <gw>`，失败回退 `-6 add`，仍失败 → 422 | `sysrc ipv6_defaultrouter=<gw>` |
| 清除 IPv6 | — | `route -6 delete default`（同上幂等） | `sysrc -x ipv6_defaultrouter` |

**先应用后持久化**：live 路由应用失败（典型：网关不在任何直连网段 → "Network is unreachable"）时返回 422，rc.conf 不动——避免「rc.conf 已写、live 未生效」的假成功。

**静态路由**（create/update/delete）：同样改为先 `route add`/`delete`（错误带 route(8) stderr，传播为 422），成功后才写 rc.conf。删除不存在的路由视为成功（幂等）。

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

通过 `useFormModal` 实现，两个字段：IPv4 网关 + IPv6 网关。预填 rc.conf 中 `defaultrouter` / `ipv6_defaultrouter` 的值。**两个字段始终随 PUT 提交**（值不变也发送）——PUT 语义是「把 rc.conf 和 live 路由都收敛到提交值」，跳过同值会让 live 与 rc.conf 漂移时（如接口变更弄丢网关后）无法通过原样保存修复。网关卡片上，configured 有值但 live 网关缺失/不一致时显示「未生效」徽标（`net.gatewayNotApplied`）。

### 配置弹窗

- 接口属性（`.config-grid` 两列）：第一行「名称 | 描述」（名称留空 = 使用默认名：默认命名的接口保持不变；**已改名的接口清空名称即回退为 driver 名**，`resolve_target_name` 解析目标——epair 半边的 driver 名是基名不带 a/b，回退到 pair 中空闲的半边，由 `ifconfig_<driver>_name` 持久化改名指令，回退后指令被删除；标签旁 `FieldHelp` 图标显示改名提示）；第二行「MTU | 扩展选项」（Options 自由填写 media/mediaopt/vlan 等额外 ifconfig 参数，`name` 指令会被自动剥离路由到名称字段）
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

### 静态路由管理

独立页面 `frontend/src/pages/StaticRoutesPage.vue`（路由 `/network/routes`）。

**rc.conf 机制**：FreeBSD 通过 `static_routes` + `route_<name>` 管理持久化静态路由：
```sh
static_routes="fwp_1 fwp_2"
route_fwp_1="-net 192.168.1.0/24 10.0.0.1"
route_fwp_2="-6 -host 2001:db8::1 fe80::1%em0"
```
`/etc/rc.d/routing` 启动时对每个 name 执行 `route add $route_<name>`。

**命名**：创建时可通过 `name` 字段自定义路由名（校验 `[a-zA-Z0-9_]+`，重名返回 409）。留空时自动生成 `net1`、`net2`、…（取已有 `net<N>` 的最大值 +1）。面板也兼容手动配置的任意 name。

**解析**（`parse_route_value`）：从 `route_<name>` 值中提取 destination、gateway、family、is_host：
- 跳过 `-6`/`-inet6` 前缀 → 标记 IPv6
- 跳过 `-net`/`-host` 前缀 → 标记路由类型
- 第一个非 `-` 开头 token = destination
- 第二个非 `-` 开头 token = gateway
- 未指定 `-net`/`-host` 时，根据 destination 是否含 `/` 自动判定

**写入**（`build_route_args`）：
- IPv6 路由前缀 `-6`
- 网络/主机路由前缀 `-net`/`-host`
- 格式：`[-6] -net|-host <dest> <gw>`

**应用**：
- 添加：`route add [-6] -net|-host <dest> <gw>`（fire-and-forget）
- 修改：先 `route delete` 旧路由，再 `route add` 新路由
- 删除：`route delete [-6] -net|-host <dest>`

**校验**：
- destination 非空，须为合法 IP 或 CIDR
- gateway 非空，须为合法 IP 地址
- family 自动检测（gateway 含 `:` → IPv6）
- is_host 自动检测（destination 无 `/` → host route）

## 文件清单

| 文件 | 说明 |
|---|---|
| `src/handlers/network.rs` | 全部 handler + getifaddrs/sysctl/ioctl 解析 + rc.conf 解析/写入/应用 |
| `src/app.rs` | 路由注册（12 条 network 路由） |
| `frontend/src/pages/NetworkPage.vue` | 接口卡片 + 路由表 + 网关 + 详情弹窗 + 配置弹窗 + 创建弹窗 |
| `frontend/src/pages/StaticRoutesPage.vue` | 静态路由 CRUD 页面 |
| `frontend/src/i18n/translations.js` | `net.*`、`staticRoutes.*` 命名空间（en + zh） |
| `frontend/src/assets/app.css` | `.net-iface`、`.config-section`、`.config-grid`、`.checkbox-row`、`.radio-pill-group`、`.form-table` 等样式 |

## 已知限制

- DHCP/SYNCDHCP 配置在 ifconfig apply 时跳过（由 dhclient 管理）
- IPv6 SLAAC/none 配置在 ifconfig apply 时跳过（仅 static 模式应用 IPv6 地址）
- 删除别名/成员需手动销毁后重建（ifconfig 无原子"替换"语义）
- 部分边缘路由的 gateway 显示为空（IPv6 零长度网关地址）
- 接口改名通过 `ifconfig_<driver>_name` 持久化（rc.d `ifnet_rename` 在配置前改名）；改名目标名校验未占用后执行，撤销改名只需清空名称字段
- apply 端点（`POST .../apply`）仅重新应用配置，不触发改名；若 rc.conf 的 `_name` 与实际 live 名不一致（外部改名漂移），apply 不负责校正
