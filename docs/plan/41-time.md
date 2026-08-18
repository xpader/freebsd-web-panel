# 模块设计：时间管理（系统时间 / 时区 / NTP 同步）

## 概述

在主菜单「系统」(`system`) 组下新增「时间」功能，提供：

1. **系统时间**——实时展示当前时间（本地 + UTC + 启动时间）、手动调整系统时间、一次性 NTP 同步（不需要 ntpd 运行）
2. **时区管理**——查看 / 修改系统时区（`/etc/localtime`），切换 RTC 模式（本地时间 / UTC），浏览 zoneinfo 区域树
3. **NTP 守护进程**——查看 ntpd 状态（启用 / 运行 / stratum / 偏移 / 对等体）、配置 `/etc/ntp.conf`（server/pool/restrict）、启停 ntpd 服务

遵循面板核心原则：面板只管理 FreeBSD 原生能力（`date`、`tzsetup`、`ntpd`、`service`），不引入运行时依赖。关闭面板后时间、时区、ntpd 配置一切照常工作。

---

## 1. 系统机制调研

### 1.1 时间读取

| 信息 | 获取方式 | 示例输出 |
|------|----------|----------|
| 本地时间 | `date` | `Fri Aug 14 11:20:42 CST 2026` |
| UTC 时间 | `date -u` | `Fri Aug 14 03:20:42 UTC 2026` |
| UTC 偏移 | `date +%z` | `+0800` |
| 时区缩写 | `date +%Z` | `CST`（有歧义） |
| 启动时间 | `sysctl -n kern.boottime` | `{ sec = 1786612041, usec = 102942 }` |
| IANA 时区名 | `readlink /etc/localtime` | `/usr/share/zoneinfo/Asia/Shanghai` |
| RTC 模式 | `ls /etc/wall_cmos_clock` | 存在→RTC 存本地时间；不存在→RTC 存 UTC |

> **时区缩写有歧义**（CST 同时表示中国标准时间和美国中部时间），程序中始终用 IANA 时区名（从 `/etc/localtime` 符号链接提取 `Asia/Shanghai`）。

### 1.2 手动设置时间

```sh
# FreeBSD date(1) 格式：MMDDhhmm[[CC]YY][.ss]
date 081411302026.00    # 设置为 2026-08-14 11:30:00（本地时间）

# date 同时更新内核时钟和硬件时钟（RTC）
# 需要 root 权限；securelevel > 1 时仅允许 ≤1 秒微调
```

设置时间后，如果 `/etc/wall_cmos_clock` 存在（RTC = 本地时间），FreeBSD 会在关机/重启时由 `adjkerntz` 自动调整。运行时修改不需要额外操作。

### 1.3 时区

```sh
# 查看当前时区（IANA 名）
readlink /etc/localtime
# → /usr/share/zoneinfo/Asia/Shanghai

# 设置时区
tzsetup Asia/Shanghai          # 等价于：
ln -sf /usr/share/zoneinfo/Asia/Shanghai /etc/localtime

# zoneinfo 目录结构
# /usr/share/zoneinfo/<Region>/<City>
# Region: Africa, America, Asia, Atlantic, Australia, Europe, Pacific, Etc, ...
```

`tzsetup <zone>` 是最安全的方式：它设置 `/etc/localtime` 符号链接，并通知内核更新时区偏移（通过 `adjkerntz -a`）。

### 1.4 RTC 模式切换（wall_cmos_clock）

| `/etc/wall_cmos_clock` | 含义 | 对应 sysctl |
|------------------------|------|-------------|
| 存在 | RTC 存本地时间（Windows 兼容） | `machdep.wall_cmos_clock=1` |
| 不存在 | RTC 存 UTC（推荐） | `machdep.wall_cmos_clock=0` |

切换到 UTC：`rm /etc/wall_cmos_clock && sysctl machdep.wall_cmos_clock=0`
切换到本地时间：`touch /etc/wall_cmos_clock && sysctl machdep.wall_cmos_clock=1 && adjkerntz -a`

### 1.5 NTP 守护进程（ntpd）

**rc.conf 变量**：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `ntpd_enable` | `NO` | 启动 ntpd |
| `ntpd_sync_on_start` | `NO` | 启动时即使偏移很大也强制同步 |
| `ntpd_config` | `/etc/ntp.conf` | 配置文件路径 |
| `ntpd_program` | `/usr/sbin/ntpd` | 二进制路径 |
| `ntpd_flags` | `""` | 额外启动参数 |
| `ntpdate_enable` | `NO` | 启动时一次性同步（已弃用，推荐用 `ntpd_sync_on_start`） |

**配置文件 `/etc/ntp.conf` 关键指令**：

```
# 时间源
server ntp.aliyun.com iburst         # 指定服务器（iburst 加速初始同步）
pool 0.freebsd.pool.ntp.org iburst   # 服务器池（ntpd 自动扩展）
tos minclock 3 maxclock 6            # 池选择参数

# 访问控制
restrict default limited kod nomodify notrap noquery nopeer
restrict 127.0.0.1
restrict ::1

# 其他
leapfile "/var/db/ntpd.leap-seconds.list"
driftfile /var/db/ntp/ntpd.drift     # 漂移文件（通过 rc.conf 默认值设置）
```

**服务控制**：`service ntpd start|stop|restart|status`

**状态查询**：

```sh
# 对等体列表（remote/refid/st/when/poll/reach/delay/offset/jitter）
ntpq -p
#  ┌ 标志位：空=未选, +=候选, *=当前同步源, -=聚类排除, x=虚假
#  remote           refid      st t when poll reach  delay  offset jitter

# 系统变量（stratum, offset, root delay 等）
ntpq -c "sysinfo"

# 漂移值
cat /var/db/ntp/ntpd.drift   # → 6.783661（ppm）
```

### 1.6 一次性 NTP 同步（无需 ntpd 运行）

```sh
# sntp（现代，推荐）—— 需要先停 ntpd（端口 123 冲突）
sntp -s pool.ntp.org

# ntpdate（弃用但仍可用）
ntpdate pool.ntp.org
```

> 面板中"立即同步"按钮：如果 ntpd 正在运行，提示用户 ntpd 会自动同步（不需要手动）；如果 ntpd 未运行，执行 `sntp -s <server>`。

---

## 2. 后端设计

### 2.1 模块结构

```
src/handlers/time.rs     — 时间、时区、NTP 的所有 handler
```

新增 `pub mod time;` 到 `src/handlers/mod.rs`。

### 2.2 数据模型

```rust
/// GET /api/time/status — 时间总览
struct TimeStatus {
    // 时间
    local_time: String,        // "2026-08-14 11:20:42 CST"（本地时间，人类可读）
    utc_time: String,          // "2026-08-14 03:20:42 UTC"
    utc_offset: String,        // "+0800"
    epoch: i64,                // Unix 时间戳（秒）
    boot_time: String,         // "2026-08-13 17:07:21"（上次启动时间）
    uptime_seconds: u64,       // 启动后经过的秒数

    // 时区
    timezone: String,          // IANA 时区名 "Asia/Shanghai"（从 /etc/localtime 提取）
    timezone_abbr: String,     // "CST"（date +%Z，显示用，标注歧义）
    rtc_local: bool,           // /etc/wall_cmos_clock 是否存在

    // NTP 状态
    ntp: NtpStatus,
}

struct NtpStatus {
    enabled: bool,             // ntpd_enable == "YES"
    running: bool,             // service ntpd 是否在运行（读 pidfile）
    sync_on_start: bool,       // ntpd_sync_on_start == "YES"
    stratum: Option<u8>,       // 来自 ntpq（运行时才有）
    offset_ms: Option<f64>,    // 当前偏移（毫秒，来自 ntpq）
    system_peer: Option<String>,// 当前同步源（如 "time.neu.edu.cn"）
    peers: Vec<NtpPeer>,       // 对等体列表
    drift: Option<f64>,        // 漂移值（ppm，来自 /var/db/ntp/ntpd.drift）
}

struct NtpPeer {
    remote: String,            // "time.neu.edu.cn" 或 IP
    refid: String,             // 参考时钟
    stratum: u8,
    state: String,             // "+" / "*" / "-" / "" （候选 / 同步源 / 排除 / 未选）
    delay_ms: f64,             // 往返延迟
    offset_ms: f64,            // 偏移
    jitter_ms: f64,            // 抖动
}

/// GET /api/time/ntp/conf — ntp.conf 解析结果
struct NtpConfig {
    servers: Vec<ServerEntry>, // server / pool 行
    restricts: Vec<String>,    // restrict 行（原样保留，高级用户编辑）
    tos: Option<String>,       // tos 行
    leapfile: Option<String>,  // leapfile 路径
    raw: String,               // 原始文件内容（用于"高级编辑"模式）
}

struct ServerEntry {
    kind: String,              // "server" | "pool"
    host: String,              // "ntp.aliyun.com"
    options: String,           // "iburst"（附加选项）
}

/// GET /api/time/zones — 可用时区列表
/// 返回 { regions: [{ name: "Asia", zones: ["Asia/Shanghai", "Asia/Hong_Kong", ...] }] }
```

### 2.3 API 设计

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/time/status` | 时间总览（时间 + 时区 + NTP 状态），前端轮询 |
| PUT | `/api/time/datetime` | 手动设置系统时间 `{ datetime: "2026-08-14T11:30:00" }`（本地时间） |
| POST | `/api/time/sync` | 一次性 NTP 同步（`sntp -s <server>`），需 ntpd 未运行 |
| GET | `/api/time/timezone` | 当前时区详情 |
| PUT | `/api/time/timezone` | 设置时区 `{ zone: "Asia/Shanghai" }`（`tzsetup`） |
| PUT | `/api/time/rtc-mode` | 切换 RTC 模式 `{ local: bool }` |
| GET | `/api/time/zones` | 可用时区列表（扫描 `/usr/share/zoneinfo/`） |
| GET | `/api/time/ntp/conf` | 读取 `/etc/ntp.conf`（结构化解析） |
| PUT | `/api/time/ntp/conf` | 写入 `/etc/ntp.conf`（原子替换 + 备份） |
| POST | `/api/time/ntp/sync-on-start` | 切换 `ntpd_sync_on_start` rc.conf 变量 |
| POST | `/api/time/ntp/enable` | 启用 ntpd（`sysrc ntpd_enable=YES` + `service ntpd start`） |
| POST | `/api/time/ntp/disable` | 禁用 ntpd（`service ntpd stop` + `sysrc ntpd_enable=NO`） |
| POST | `/api/time/ntp/restart` | 重启 ntpd |

### 2.4 实现要点

#### 手动设置时间

```rust
// 前端传入 ISO 8601 本地时间："2026-08-14T11:30:00"
// 转换为 date(1) 格式 MMDDhhmmCCYY.ss
// 执行: date MMDDhhmmCCYY.ss（spawn_blocking）
// date 同时更新内核时钟和硬件 RTC，无需额外操作
```

安全：`date` 格式严格校验（正则 `^\d{2}\d{2}\d{2}\d{2}(\d{4})?(\.\d{2})?$`），拒绝非法输入。securelevel > 1 时仅允许微调（≤1 秒），面板应检测并提示。

#### 一次性 NTP 同步

```rust
// 1. 检查 ntpd 是否在运行
// 2. 如果运行中 → 返回提示 "ntpd 正在运行，会自动同步"
// 3. 如果未运行 → spawn_blocking 执行 sntp -s <server>
//    sntp 输出同步结果（偏移量等），解析返回前端
```

#### 时区设置

```rust
// tzsetup <zone>（spawn_blocking）
// tzsetup 内部：ln -sf /usr/share/zoneinfo/<zone> /etc/localtime + adjkerntz -a
// 校验 zone 合法性：必须匹配 ^[A-Za-z_0-9/+.-]+$ 且文件存在于 /usr/share/zoneinfo/
```

#### ntpd 状态查询

```rust
// enabled: sysrc::get("ntpd_enable") == Some("YES")
// running: 读 pidfile /var/db/ntp/ntpd.pid，检查 PID 存活
// peers: spawn_blocking 执行 ntpq -p，解析固定列宽输出
//   - 跳过 .POOL. 行（虚拟池条目），或标记为 "pool" 类型
//   - 行首标志位提取：+ * - x 空格
//   - 数值字段：delay/offset/jitter 按 ms 换算
// drift: 直接读 /var/db/ntp/ntpd.drift 文件（无子进程）
```

`ntpq -p` 输出是固定列宽，但 `-w`（宽模式）会折行。**默认不用 `-w`**，普通模式下主机名被截断为 15 字符。需 `ntpq -c "associations"` + `ntpq -c "rv <associd>"` 取完整信息，或接受截断。对面板展示足够（15 字符 + IP 地址通常完整）。

#### ntp.conf 解析与写入

**解析**：逐行扫描，按首关键字分类：
- `server` / `pool` → `ServerEntry { kind, host, options }`
- `restrict` → 原样存入 restricts 数组
- `tos` → 存储原始行
- `leapfile` → 提取路径
- `#` 或空行 → 保留在 raw 中

**写入**：采用"编辑"策略而非"全量重建"——只修改 `server`/`pool` 行（用户最常操作的），其余行原样保留。具体：
1. 读取原始文件全部行
2. 标记并删除所有 `server`/`pool` 开头的行
3. 在原 `server`/`pool` 块的位置插入新的 server/pool 行
4. 原子写入（先写临时文件 → rename）
5. 保留所有注释、restrict、tos 等配置不变

这样避免破坏用户自定义的安全策略（restrict）和高级配置。

### 2.5 命令封装

复用现有基础设施：

| 操作 | 方式 | 依据 |
|------|------|------|
| 读 rc.conf（ntpd_enable 等） | `sysrc::get()` / `sysrc::is_yes()` | 直接读文件，无子进程 |
| 写 rc.conf | `sysrc::set_async()` | sysrc 子进程 |
| service 控制 | `cmd::run()` | spawn_blocking |
| 设置时间 | `cmd::run("date", &[formatted])` | spawn_blocking |
| sntp 同步 | `cmd::run()` | spawn_blocking |
| tzsetup | `cmd::run("tzsetup", &[zone])` | spawn_blocking |
| ntpq 查询 | `cmd::run("ntpq", &["-p"])` | spawn_blocking |
| 读 ntp.conf | `std::fs::read_to_string`（spawn_blocking 内） | 无子进程 |
| 写 ntp.conf | 原子写入（临时文件 + rename） | 无子进程 |
| 读 drift 文件 | `std::fs::read_to_string`（spawn_blocking 内） | 无子进程 |
| 读 boottime | `sysinfo.rs` 现有 `sysctl(3)` 或 `cmd::run("sysctl", &["-n", "kern.boottime"])` | — |
| 读 /etc/localtime 链接 | `std::fs::read_link` | 无子进程 |
| 扫描 zoneinfo | `std::fs::read_dir` 递归（spawn_blocking 内） | 无子进程 |

### 2.6 审计日志

所有写操作记录审计日志（`audit::record`）：
- 设置时间 → `"set datetime to 2026-08-14T11:30:00"`
- 一次性同步 → `"sntp sync with pool.ntp.org"`
- 设置时区 → `"set timezone to Asia/Shanghai"`
| 切换 RTC 模式 → `"set RTC mode to local/UTC"`
- 启用/禁用 ntpd → `"enable/disable ntpd"`
- 修改 ntp.conf → `"update ntp.conf servers"`
- 重启 ntpd → `"restart ntpd"`

---

## 3. 前端设计

### 3.1 路由与菜单

**菜单**：`frontend/src/lib/menu.js` → `system` 组新增一项：

```js
{ path: '/time', labelKey: 'nav.time', icon: 'fa-solid fa-clock' },
```

**路由**：`frontend/src/router/index.js` 新增：

```js
{ path: 'time', name: 'time', component: () => import('../pages/TimePage.vue') },
```

### 3.2 页面结构（`TimePage.vue`）

页面分为三个区域（卡片式布局，不使用多级菜单）：

```
┌─────────────────────────────────────────────────────────┐
│  系统时间                                                 │
│  ┌──────────────────┐  ┌──────────────────┐             │
│  │ 本地时间           │  │ UTC 时间          │  [手动调整] │
│  │ 2026-08-14        │  │ 2026-08-14        │  [立即同步] │
│  │ 11:20:42 CST      │  │ 03:20:42 UTC      │             │
│  └──────────────────┘  └──────────────────┘             │
│  启动时间: 2026-08-13 17:07:21 (运行 18小时13分)           │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│  时区                                                    │
│  当前时区: Asia/Shanghai (CST, UTC+08:00)                 │
│  硬件时钟: UTC  [切换为本地时间]                            │
│  [更改时区] → 弹出时区选择对话框（区域 → 城市 两级选择）      │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│  NTP 同步 (ntpd)                              [启用] [停止] [重启] │
│  状态: ● 运行中   Stratum: 2   偏移: -1.0ms   同步源: time.neu.edu.cn │
│  □ 启动时强制同步 (ntpd_sync_on_start)                     │
│                                                          │
│  NTP 服务器:                          [+ 添加服务器]        │
│  ┌──────────────────────────────────────────────────┐    │
│  │ ○ pool  0.freebsd.pool.ntp.org    iburst   [删除] │    │
│  │ ● server ntp.aliyun.com           iburst   [删除] │ ← ● = 同步源 │
│  │ ○ server ntp1.aliyun.com          iburst   [删除] │    │
│  └──────────────────────────────────────────────────┘    │
│  [保存配置]                                                │
│                                                          │
│  对等体状态                                    [刷新]       │
│  ┌────────────────┬──────┬───────┬────────┬────────┬─────┐│
│  │ 远程主机        │层级  │ 延迟   │ 偏移   │ 抖动   │状态 ││
│  ├────────────────┼──────┼───────┼────────┼────────┼─────┤│
│  │*time.neu.edu.cn│  1   │38.4ms │-1.0ms  │0.97ms  │同步 ││
│  │+203.107.6.88   │  2   │26.5ms │+2.7ms  │1.08ms  │候选 ││
│  └────────────────┴──────┴───────┴────────┴────────┴─────┘│
│  漂移: 6.78 ppm                                          │
└─────────────────────────────────────────────────────────┘
```

### 3.3 交互细节

**手动调整时间**——用 `useFormModal` 弹出日期时间选择器（`<input type="datetime-local">`），默认填入当前时间。确认后调用 `PUT /api/time/datetime`。**使用确认弹窗**（`useConfirm`）二次确认（修改系统时间是高风险操作）。

**立即同步**——调用 `POST /api/time/sync`。如果 ntpd 正在运行，后端返回提示信息，前端用 `useAlert` 展示。成功用 toast。

**更改时区**——用 `useFormModal` 弹出两级选择器（区域下拉 → 城市下拉），数据来自 `GET /api/time/zones`。确认后调用 `PUT /api/time/timezone`。用 `useConfirm` 二次确认。

**NTP 服务器编辑**——前端维护可编辑列表（添加 server/pool 行、删除行、修改 host/options），保存时整体提交 `PUT /api/time/ntp/conf`。保存成功后提示是否重启 ntpd 使配置生效。

**实时刷新**——系统时间区域每秒自动更新（前端 `setInterval` 1000ms，基于 epoch + 1 秒递增，避免每秒请求 API）。NTP 状态区域每 10 秒轮询 `GET /api/time/status` 的 ntp 部分。`onUnmounted` 清理定时器。

### 3.4 i18n

新增翻译键到 `frontend/src/i18n/translations.js` 的 `nav` 命名空间（`nav.time`）和新建 `time` 命名空间。遵守规范：通用词（如 enable/disable/restart/save/delete/refresh）复用 `common`，时间专用词建 `time` 命名空间。

---

## 4. 安全考量

1. **date 命令注入防护**——时间格式严格正则校验，不拼接 shell。使用 `Command::new().arg()` 传参。
2. **时区名校验**——`^[A-Za-z0-9_/+.-]+$`，且验证 `/usr/share/zoneinfo/<zone>` 文件存在。拒绝路径穿越（`..`、绝对路径）。
3. **ntp.conf 写入**——原子替换（临时文件 + rename），保留备份。不做 shell 拼接。server host 校验 `^[a-zA-Z0-9._:-]+$`。
4. **securelevel 限制**——`securelevel > 1` 时时间修改受限（仅 ≤1 秒微调）。面板应检测 securelevel 并提示用户（只读展示，不做额外限制——内核本身会拒绝）。
5. **ntpd 停止后再同步**——sntp 与 ntpd 竞争端口 123，必须确保 ntpd 已停止。后端先检查运行状态。

---

## 5. 实现里程碑

### M1 — 后端只读 API（时间 + 时区 + NTP 状态查询）

- `GET /api/time/status`（时间 + 时区 + ntpd enabled/running + ntpq peers + drift）
- `GET /api/time/ntp/conf`（ntp.conf 解析）
- `GET /api/time/zones`（zoneinfo 扫描）
- 单元测试：ntpq 输出解析、ntp.conf 解析、date 格式校验

### M2 — 后端写操作

- `PUT /api/time/datetime`（手动设置时间）
- `POST /api/time/sync`（一次性 sntp 同步）
- `PUT /api/time/timezone`（tzsetup）
- `PUT /api/time/rtc-mode`（wall_cmos_clock 切换）
- `PUT /api/time/ntp/conf`（ntp.conf 原子写入）
- `POST /api/time/ntp/{enable,disable,restart,sync-on-start}`
- 全部带审计日志

### M3 — 前端页面

- `TimePage.vue`（三区域卡片布局）
- 时区选择器（useFormModal + 两级下拉）
- 手动时间调整（datetime-local + 二次确认）
- NTP 服务器列表编辑 + 保存
- 实时刷新（前端定时器 + API 轮询）
- i18n 翻译键

### M4 — 集成验证

- 菜单 / 路由接线
- 端到端：设置时间 → 验证 `date` 输出一致
- 端到端：切换时区 → 验证 `/etc/localtime` 链接变更
- 端到端：编辑 NTP 服务器 → 重启 ntpd → `ntpq -p` 显示新服务器
- 审计日志验证

---

## 6. 已知限制 / 不实现

- **不做时间同步历史图表**——面板监控（`monitor.rs`）已有 CPU/内存图表，NTP 偏移不纳入时序采集（需求不足，避免增加调度器负担）。可在未来扩展。
- **不做 ntp.conf 的高级语法编辑器**——restrict 行原样保留，不提供 GUI 编辑（安全策略变化多端，GUI 化反而增加误配风险）。高级用户通过 Web 终端编辑。
- **不支持 OpenNTPD / chrony**——仅管理 FreeBSD 自带的 ntpd（`/usr/sbin/ntpd`）。如果用户安装了 chrony 等替代品，面板不管理（避免冲突）。
- **不做 RTC 硬件诊断**——仅切换 `/etc/wall_cmos_clock` 标记文件，不诊断硬件时钟电池等问题。
