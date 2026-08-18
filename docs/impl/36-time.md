# 时间管理（系统时间 / 时区 / NTP）

## 概述

在主菜单「系统」下提供时间管理功能，包括：
- **系统时钟**：实时显示本地时间和 UTC 时间、手动设置系统时间、一次性 NTP 同步
- **时区**：查看/修改 IANA 时区（通过 `tzsetup`）、切换硬件 RTC 模式（UTC ↔ 本地时间）
- **NTP 守护进程**：查看 ntpd 状态（stratum/偏移/对等体/漂移）、编辑 `/etc/ntp.conf` 的 server/pool 行、启停/重启 ntpd

面板只调用 FreeBSD 原生工具（`date`、`tzsetup`、`ntpd`、`sntp`、`ntpq`、`sysctl`、`adjkerntz`），不引入运行时依赖。

## 实现细节

### 源码位置

| 文件 | 说明 |
|------|------|
| `src/handlers/time.rs` | 全部 handler（读/写）+ ntpq 解析 + ntp.conf 解析/重建 + zoneinfo 扫描 |
| `src/app.rs` | 路由注册（11 条路由） |
| `frontend/src/pages/TimePage.vue` | 前端单页面（系统时间 + 时区 + NTP 三区域） |

### 时间读取

使用 `chrono`（`Local`/`Utc`）获取当前时间。`chrono::Local` 通过 libc 的 `localtime(3)` 自动读取 `/etc/localtime`。

UTC 偏移通过 `chrono::Local::now().offset().fix().local_minus_utc()` 计算，格式化为 `+0800` 形式。

IANA 时区名通过 `readlink("/etc/localtime")` 提取，剥离 `/usr/share/zoneinfo/` 前缀。若 `/etc/localtime` 不是符号链接（被 `tzsetup` 复制为普通文件），则使用 `canonicalize` 回退，最终回退为 `"Unknown"`。

RTC 模式通过检测 `/etc/wall_cmos_clock` 文件是否存在判断。

### 手动设置时间

前端传入 ISO 8601 本地时间（`2026-08-14T11:30:00`），后端用 `chrono::NaiveDateTime` 解析，格式化为 FreeBSD `date(1)` 操作数格式 `CCYYMMDDHHMM.ss`（如 `202608141130.00`），通过 `cmd::run_sync("/bin/date", &[formatted])` 执行。

`date(1)` 同时更新内核时钟和硬件 RTC。securelevel > 1 时内核仅允许 ≤1 秒微调（面板不做额外限制，内核会自行拒绝）。

### 一次性 NTP 同步

调用 `/usr/sbin/sntp -s <server>`。**若 ntpd 正在运行则拒绝**（返回 `BadRequest`），因为 ntpd 已经自动管理同步。

### 时区设置

调用 `tzsetup <zone>`。验证 zone 名：不允许路径穿越（`..`、绝对路径），仅允许 `[A-Za-z0-9_/+.-]`，且 `/usr/share/zoneinfo/<zone>` 必须存在。

### RTC 模式切换

- 切换为 UTC：`rm /etc/wall_cmos_clock` + `sysctl machdep.wall_cmos_clock=0`
- 切换为本地时间：`touch /etc/wall_cmos_clock` + `sysctl machdep.wall_cmos_clock=1` + `adjkerntz -a`

**UI 说明**：RTC 行标签旁有 ⓘ 图标——悬停一句话提示（`rtcModeHint`），点击弹出说明（`rtcHelp`，i18n 双语）。文案：模式 = 开机时内核解读 RTC 值的约定（直接视为 UTC / 视为本地时间经 adjkerntz 按 /etc/localtime 换算）；系统时钟恒为 UTC 并由 ntpd 校准，模式只影响初始读取；模式与存储值不符时开机时间偏差一个时区；标准做法 UTC，本地时间仅多系统引导且另一系统按本地时间读取 RTC 时使用。切换动作 = 创建/删除 `/etc/wall_cmos_clock`（重启保持）+ `machdep.wall_cmos_clock`（立即生效）；切到本地时间 adjkerntz 立即重写 RTC，切到 UTC 不重写——确认弹窗提醒立即同步一次。alert 弹窗消息支持 `\n\n` 分段（`DialogHost.vue` 的 `white-space:pre-line`）。

### ntpd 状态查询

| 信息 | 获取方式 |
|------|----------|
| enabled | `sysrc::is_yes("ntpd_enable")`（直接读 rc.conf，无子进程） |
| running | 读 pidfile `/var/db/ntp/ntpd.pid` → `libc::kill(pid, 0)` 检查进程存活 |
| peers | `ntpq -p` 输出解析（仅在 ntpd 运行时调用） |
| drift | 直接读 `/var/db/ntp/ntpd.drift` 文件（无子进程） |
| sync_on_start | `sysrc::is_yes("ntpd_sync_on_start")` |

### ntpq 输出解析

`ntpq -p` 输出为固定列宽表格。解析逻辑：

1. 跳过表头（`===` 分隔线之前）
2. 每行首字符是状态指示符：`*`=同步源, `+`=候选, `-`=排除, `x`=虚假, `#`=备份
3. `line[1..].split_whitespace()` 取 10 个字段：remote, refid, st, type, when, poll, reach, delay, offset, jitter
4. 跳过 `refid == ".POOL."` 的虚拟池条目
5. `*` 标记的 peer 即系统同步源，其 stratum+1 = 系统层级

### ntp.conf 解析与写入

**解析**（`parse_ntp_conf`）：逐行扫描，`server`/`pool` 开头的行解析为 `ServerEntry { kind, host, options }`。其他行（restrict、tos、leapfile、注释）保留在 `raw` 中。

**写入**（`rebuild_ntp_conf`）：编辑策略，非全量重建。
1. 记录第一个 server/pool 行的位置
2. 删除所有 server/pool 行
3. 在原位置插入新的 server/pool 行
4. 其余行（restrict、tos、leapfile、注释）原样保留

原子写入：先写 `/etc/ntp.conf.fwp-tmp` → `rename` 覆盖。写入前通过共享模块 `src/backup.rs` 的 `backup_file` 把当前 ntp.conf 快照到统一备份目录 `/var/db/fwp/conf_backup/ntp.conf.<unix-秒时间戳>`（保留最近 5 份，失败仅告警不阻断）。

### zoneinfo 扫描

递归扫描 `/usr/share/zoneinfo/`，排除 `posix/`、`right/`、`src/`、`locale/`、`SystemV/` 目录和 `.tab`/`.list`/`Factory` 文件。结果按区域分组（`BTreeMap<String, Vec<String>>`），每个 zone 名包含完整路径（如 `America/Argentina/Buenos_Aires`）。

### ntpd 服务控制

| 操作 | 实现 |
|------|------|
| 启用 | `sysrc ntpd_enable=YES` + `service ntpd start` |
| 禁用 | `service ntpd stop` + `sysrc ntpd_enable=NO` |
| 重启 | `service ntpd restart` |

## API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/time/status` | 时间总览（时钟 + 时区 + NTP 状态 + peers） |
| PUT | `/api/time/datetime` | 手动设置系统时间 `{ datetime: "2026-08-14T11:30:00" }` |
| POST | `/api/time/sync` | 一次性 NTP 同步 `{ server?: "pool.ntp.org" }`（ntpd 运行时拒绝） |
| PUT | `/api/time/timezone` | 设置时区 `{ zone: "Asia/Shanghai" }` |
| PUT | `/api/time/rtc-mode` | 切换 RTC 模式 `{ local: bool }` |
| GET | `/api/time/zones` | 可用时区列表（按区域分组） |
| GET | `/api/time/ntp/conf` | 读取 ntp.conf（结构化 server 列表 + raw） |
| PUT | `/api/time/ntp/conf` | 更新 ntp.conf server/pool 行 `{ servers: [...] }` |
| PUT | `/api/time/ntp/sync-on-start` | 切换 `ntpd_sync_on_start` `{ enabled: bool }` |
| POST | `/api/time/ntp/enable` | 启用 + 启动 ntpd |
| POST | `/api/time/ntp/disable` | 停止 + 禁用 ntpd |
| POST | `/api/time/ntp/restart` | 重启 ntpd |

## 外部依赖

### 系统命令

| 命令 | 用途 |
|------|------|
| `/bin/date` | 设置系统时间（操作数格式 `CCYYMMDDHHMM.ss`） |
| `/usr/sbin/tzsetup` | 设置时区 |
| `/usr/sbin/sntp` | 一次性 NTP 同步 |
| `/usr/bin/ntpq` | 查询 ntpd 对等体状态 |
| `/sbin/sysctl` | 切换 `machdep.wall_cmos_clock` |
| `/sbin/adjkerntz` | 调整内核时区偏移（RTC 本地时间模式） |
| `/usr/sbin/service` | ntpd 启停重启 |
| `/usr/sbin/sysrc` | 读写 rc.conf（通过 `sysrc.rs` 封装） |

### Rust crate

- `chrono`（`clock` feature）—— 本地/UTC 时间、偏移计算、日期解析
- `libc` —— `kill(pid, 0)` 检查进程存活
- `sysctl`（通过 `sysinfo.rs`）—— `kern.boottime` 读取

## 配置项

无独立配置项。相关 rc.conf 变量：

| 变量 | 说明 |
|------|------|
| `ntpd_enable` | ntpd 启用状态 |
| `ntpd_sync_on_start` | 启动时强制同步 |

## 前端

### 页面结构

单页面三区域卡片布局：
1. **系统时间**：双时钟卡片（本地/UTC）+ 启动时间 + 运行时长，操作按钮（设置时间、立即同步）
2. **时区**：当前时区信息 + RTC 模式显示与切换 + 更改时区按钮（两级选择：区域 → 城市）
3. **NTP 同步**：ntpd 状态行（stratum/偏移/同步源/漂移）+ 服务控制按钮（启动/停止/重启）+ sync-on-start 开关（即时保存，独立于服务器配置）+ 两个弹窗按钮——「NTP 服务器」（编辑服务器列表，显式保存）与「NTP 对等体」（只读状态表）

### 实时时钟

前端从 API 获取 `epoch` 后，每秒 `setInterval` 递增 epoch 并格式化显示。本地时间格式化使用服务器返回的 `utc_offset`（不依赖浏览器时区）。NTP 状态每 15 秒轮询 `GET /api/time/status`。`onUnmounted` 清理定时器。

### 设计文档

详见 `docs/plan/41-time.md`。

## 已知限制

- **timezone_abbr 可能显示偏移而非缩写**：chrono 的 `%Z` 格式在某些系统配置下返回 `+08:00` 而非 `CST`，但这更准确（无歧义）。
- **ntp.conf 仅编辑 server/pool 行**：restrict 行不提供 GUI 编辑（安全策略变化多端，GUI 化增加误配风险）。高级用户通过 Web 终端编辑。
- **不支持 OpenNTPD / chrony**：仅管理 FreeBSD 自带的 ntpd。
- **一次性同步需 ntpd 已停止**：sntp 与 ntpd 功能重叠，运行时拒绝以避免冲突。
