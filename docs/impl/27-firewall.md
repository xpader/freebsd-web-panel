# 27 — 防火墙（双驱动 ipfw / pf）

## 概述

在 Web 面板中提供统一的防火墙管理界面，支持 FreeBSD 两大防火墙引擎 ipfw 和 pf。采用两级菜单结构，分为三个子页面：**规则管理**（`/firewall/rules`）、**IP 名单**（`/firewall/tables`）、**设置**（`/firewall/settings`）。用户可选择驱动、切换驱动、以结构化表单增删改规则、维护可复用的 IP 地址名单并在规则中引用、切换黑白名单模式。规则统一存储在 SQLite 中，切换引擎时自动生成对应语法的配置文件。

**关键设计决策**：
- 规则存储与引擎解耦——一套规则在 DB 中，切换引擎时重新生成对应语法的 `/etc/ipfw.rules` 或 `/etc/pf.conf`
- IP 名单（tables）统一存储 DB，生成配置时映射为各引擎的 table 语法
- 初始化只写配置文件和 rc.conf，不自动启用防火墙（防断网）
- 黑白名单模式通过规则实现，不依赖 boot-time tunable

## 实现细节

### 数据结构

**Rust 结构体**（`src/firewall_gen.rs`）：

```rust
enum FirewallDriver { Ipfw, Pf }
enum FirewallMode { Whitelist, Blacklist }  // 白名单=默认拒绝，黑名单=默认放行
enum RuleAction { Allow, Deny, Reject }
enum RuleDirection { In, Out }
enum RuleProtocol { Tcp, Udp, Icmp, Icmpv6, Any }
enum AddressKind { Any, Single, Cidr, Me, Table }
// Table: value 存储名单名称，生成时映射为 table 引用

struct IpTable {
    id, name, description,
    entries: Vec<IpTableEntry>,  // { id, table_id, address, created_at }
    created_at, updated_at,
}

struct FirewallRule {
    id, position, enabled,
    action, direction, protocol,
    source: AddressSpec,        // { kind, value }
    source_port: Option<String>,
    destination: AddressSpec,
    destination_port: Option<String>,
    interface: Option<String>,
    log: bool,
    icmp_type: Option<String>,  // ICMP 类型（如 "8"=echo-request）
    description: Option<String>,
    created_at, updated_at,
}
```

### SQLite 表

```sql
CREATE TABLE firewall_rules (
    id, position, enabled,
    action, direction, protocol,
    src_kind, src_value, src_port,
    dst_kind, dst_value, dst_port,
    interface, log, icmp_type, description,
    created_at, updated_at
);
-- 所有规则统一存储，切换引擎时共用同一套规则，不按引擎分表

CREATE TABLE firewall_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- keys: 'active_driver' → 'ipfw'|'pf'
--       'mode'          → 'whitelist'|'blacklist'

CREATE TABLE firewall_tables (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE firewall_table_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    table_id INTEGER NOT NULL,
    address TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (table_id) REFERENCES firewall_tables(id) ON DELETE CASCADE
);
```

> **注意**：规则与引擎完全解耦，所有规则统一存储。切换引擎时共用同一套规则，不按引擎分表。

### 规则生成

**ipfw**（`generate_ipfw`）— 生成原生规则文件 `/etc/ipfw.rules`（pathname 模式）：
- 文件格式为 ipfw 原生语法，通过 `ipfw -q /etc/ipfw.rules` 加载（每行内容作为 ipfw 参数，`#` 为注释标记）
- 首行为 `-f flush`（清除所有现有规则）
- IP 名单定义：`table NAME flush` + `table NAME add ADDR`
- 名单引用语法：`table(NAME)`（括号无需转义，非 shell 语法）
- 规则编号从 00100 开始，步进 100
- 白名单模式在规则 65534 前加 `add 65000 allow ip from any to any out keep-state`（放行出站，否则启用后 HTTP 响应出站包被 deny，导致连接挂起）
- 白名单模式末尾加 `add 65534 deny ip from any to any`
- 黑名单模式末尾加 `add 65534 allow ip from any to any`
- allow 规则自动加 `keep-state`（deny/reject 规则不加）
- ICMP 类型用数字（ipfw 不接受名字）
- 多端口用逗号分隔：`80,443,8080-8090` 直接传递（ipfw 的花括号 `{ }` 是规则级 OR-list，不是端口列表）

**pf**（`generate_pf`）— 生成 `/etc/pf.conf`：
- 先生成 IP 名单声明：`table <NAME> { addr1, addr2 }` 或 `table <NAME> persist`（空名单）
- 名单引用语法：`<NAME>`
- 白名单模式：`set skip on lo0` + `block all` + `pass out quick all flags any keep state (sloppy)`
- 黑名单模式：`set skip on lo0`（pf 默认放行）
- 用户规则使用 `quick`（首个匹配生效，与 ipfw 语义一致）
- **PF 规则关键字顺序**（`pf.conf(5)` 强制）：`action [direction] [log] [quick] [on interface] [af] [proto] from ... to ...`。`on interface` 必须在 `inet`/`inet6` 之前，否则 `pfctl -n -f` 报语法错误。生成器通过 `parts: Vec<String>` 按此顺序构建，不单独拼接 log/interface
- `set skip on lo0` 必须在 `block all` 前面
- **状态保持（keep state）自动判断**：allow + TCP → `flags any keep state (sloppy)`；allow + UDP/ICMP → 裸 `keep state`；allow + `any` 协议 → 裸 `keep state`（对无连接协议无实际效果，但不影响正确性）；deny/reject → 不加。
  - **`flags any`**：让 PF 匹配任意 TCP 标志位的包（包括已有连接的 ACK/PSH），确保 PF 在连接中途启用时非 SYN 包也能匹配规则、创建状态，不被 `block all` 丢弃。
  - **`(sloppy)`**：使用 PF 的宽松 TCP 状态跟踪（`pf_tcp_track_sloppy`）。标准状态跟踪（`pf_tcp_track_full`）在为已有连接的非 SYN 包创建状态后，会将连接状态设为 `PFTM_TCP_OPENING`（30 秒超时），导致状态在 30 秒内过期。`sloppy` 模式有专门的半连接处理——收到 ACK 时直接将两端设为 `TCPS_ESTABLISHED`（pf.c:6988-6998），超时变为 24 小时。白名单出站基准规则 `pass out quick all flags any keep state (sloppy)` 同理。
- ICMP 类型用数字（FreeBSD pf 不接受名字）
- 地址族判定优先级：ICMP → inet/inet6（`icmp-type` 必需）；Table 引用 → 省略 AF（支持混合 IPv4/IPv6）；其余按地址内容检测 v4/v6
- 离散/连续端口转换：`80,443,8080-8090` → `{ 80, 443, 8080:8090 }`

### ICMP 类型处理

前端下拉框显示友好名称（如 `echo-request (8) Ping`），存储和生成时用数字。`icmp_name_to_number()` 函数将名称映射为数字。

### 端口格式转换

用户输入统一格式 `80,443,8080-8090`（逗号分隔，短横线表示范围），生成器转换为各引擎语法：
- ipfw: `port_to_ipfw()` → `80,443,8080-8090`（逗号分隔直接传递）
- pf: `port_to_pf()` → `{ 80, 443, 8080:8090 }`（pf 范围用冒号）

### 初始化流程

`init_ipfw(mode, rules, tables, nat_rules)`：
1. sysrc 设置 `firewall_enable=YES`、`firewall_type=/etc/ipfw.rules`、`firewall_quiet=YES`、`firewall_logging=YES`
2. sysrc 删除 `firewall_script`（回退到 `/etc/rc.firewall`，由其 `*)` 分支执行 `ipfw -q ${firewall_type}` 加载规则）
3. `kldload ipfw`（如未加载）
4. **立即 `sysctl net.inet.ip.fw.enable=0`**——kldload 默认启用 ipfw 且默认规则为 deny，不显式禁用会断网
5. 生成配置文件（`atomic_write` 到 `/etc/ipfw.rules`）
6. **不加载规则、不启用防火墙**——用户手动 Apply + Enable

`init_pf(mode, rules, tables)`：
1. sysrc 设置 `pf_enable=YES`、`pf_rules=/etc/pf.conf`
2. `kldload pf`（如未加载）
3. 生成配置文件
4. **不加载规则、不启用防火墙**

### 启用/禁用

**enable**：确保当前驱动 `*_enable=YES`，对方 `*_enable=NO`，清除 staging，重生成配置文件，备份配置文件，创建 pending 记录，然后启用防火墙，最后 spawn 倒计时定时器。
- ipfw: `service ipfw start`（rc.d 脚本自动 `kldload ipfw` + rc.firewall 执行 `ipfw -q /etc/ipfw.rules` + `sysctl net.inet.ip.fw.enable=1`）
- pf: `service pf start`（rc.d 脚本自动 `kldload pf` + `pfctl -F all` + `pfctl -f /etc/pf.conf` + `pfctl -eq`）
- **两个引擎都通过 `service start`**：rc.subr 的 `required_modules` 自动加载内核模块，解决重启后模块未加载的问题
- **防锁死**：enable 必定触发倒计时（`was_enabled=false`，回滚时禁用防火墙）

**disable**：运行时禁用 + rc.conf 设为 `NO`，同时清除 staging 文件（若有未提交的变更则丢弃）。
- ipfw: `service ipfw stop`（`sysctl net.inet.ip.fw.enable=0`）+ `sysrc firewall_enable=NO`
- pf: `pfctl -d`（fire-and-forget）+ `sysrc pf_enable=NO`

**status handler**：`is_firewall_enabled` / `module_loaded` 通过 `spawn_blocking` 调用，避免阻塞 async 线程。`pending_apply` 字段反映 staging 文件是否存在。

### 切换引擎

1. 禁用旧驱动（`deactivate_ipfw`/`deactivate_pf`——运行时禁用 + rc.conf 设 NO）
2. 初始化新驱动（`init_*`；ipfw 的 `kldload` 后会立即显式禁用）
3. 在新驱动仍禁用时加载规则（`apply_*`）
4. 更新 DB `active_driver`
5. 不自动启用——返回 `enabled=false`

### 切换黑白名单模式

1. 更新 DB `mode`
2. 重新 apply 规则（ipfw 和 pf 都需要重新生成+加载）
3. ipfw 模式切换不写 loader.conf/sysctl.conf——通过规则 65534 实现

### 暂存机制（staging）——防火墙已启用时的规则修改

**问题**：防火墙已启用时修改规则后 apply，如果新规则阻断了管理连接，需要回滚到旧规则。但旧规则在 apply 前被新规则覆盖，DB 也已更新——无法恢复。

**设计**：引入 staging 文件（`/var/db/fwp/firewall_staging.json`），类似快照/MVCC：

- **防火墙未启用**：修改规则 → 直接写 DB + 重生成配置文件，无暂存，无「应用变更」按钮
- **防火墙已启用**：修改规则 → 写 staging 文件（DB 不变），列表显示 staging 内容（若有）
  - 「应用变更」按钮 = staging 文件存在时显示
  - **确认** → staging 全量写入 DB（`replace_all_rules` + `replace_all_tables`）+ 删除 staging
  - **恢复** → 删除 staging（DB 从未改动）+ 恢复备份的配置文件

**staging 文件格式**：全量快照（所有规则 + 所有表），而非增量变更日志。选择快照而非 diff 的原因：
1. 简单可靠——confirm 时 `replace_all`，rollback 时删文件，不需要处理增删改的逆操作
2. 无累积误差——多次修改 staging 文件只反映最终状态，不会因为操作日志 replay 出错
3. 文件极小——几十条规则序列化只有几 KB

**全量写入代价**：confirm 时执行 `DELETE FROM firewall_rules` + 批量 `INSERT`，同样处理 `firewall_tables` 和 `firewall_table_entries`。SQLite 单事务内毫秒级完成。操作频率低（仅用户确认时触发一次），可接受。

**CRUD 行为差异**（`handlers/firewall.rs`）：

| 操作 | 防火墙未启用 | 防火墙已启用 |
|---|---|---|
| create_rule | DB INSERT + regen config | staging 新增（内存 Vec 操作）|
| update_rule | DB UPDATE + regen config | staging 修改（内存 Vec 操作）|
| delete_rule | DB DELETE + regen config | staging 删除（内存 Vec 操作）|
| toggle_rule | DB UPDATE + regen config | staging 修改 enabled 字段 |
| reorder_rules | DB UPDATE position + regen config | staging 修改 position |
| 表 CRUD | 同上 | 同上 |

**`effective_state()`**：读取规则/表时，优先返回 staging（若存在），否则返回 DB。所有 list_rules、list_tables、config 预览、apply 都使用此函数。

**`regen_config()`**：从 DB 读取规则 + 表，调用 `write_config_only()` 生成配置文件但不加载到内核。防火墙未启用时使用，确保配置文件始终与 DB 一致。PF 路径仅 `atomic_write` 不做 `pfctl -n` 校验（PF 模块可能未加载，`/dev/pf` 不存在时校验会卡住）；ipfw 路径同理只写文件。校验延迟到 apply/enable 时执行。

> **DB 锁注意事项**：`regen_config()` 内部会 `state.db.lock().await`，因此调用方**必须先释放自己持有的 DB 锁**后再调用。由于 `tokio::sync::Mutex` 不可重入，如果调用方在持有锁时直接调用 `regen_config()`，会导致死锁。正确模式：`{ let conn = state.db.lock().await; /* DB ops */ }` 块作用域释放锁后再 `regen_config().await`。

### Apply 流程

1. 检查是否有 pending confirm（如有则拒绝——`409 Conflict`）
2. 从 `effective_state()` 读取规则（staging 若存在，否则 DB）
3. 检查防火墙当前是否已启用
4. **备份当前配置文件内容**（读 `/etc/pf.conf` 或 `/etc/ipfw.rules` 全文）
5. 调用生成器生成配置文件内容
6. 原子写入配置文件（临时文件 → rename）
7. 加载规则（ipfw: 先 `ipfw -n -q` 语法验证，再 `ipfw -q /etc/ipfw.rules` 加载；pf: `service pf reload` = `pfctl -n -f` 验证 + `pfctl -f` 加载，然后 `pfctl -F states` 刷新状态表——强制断开旧连接，使新规则立即对所有连接生效）
8. 清除 `rules_dirty` 标记（历史遗留，暂存机制已接管 pending_apply 语义）
9. **如果防火墙已启用**：
   - 写入 `/var/db/fwp/firewall_pending.json`（备份配置 + `was_enabled=true` + `expires_at=now+60s`）
   - **staging 文件保留不删除**——confirm 时提交到 DB，rollback 时丢弃
   - spawn 倒计时任务（`tokio::spawn(sleep 60s)` → 检查 pending → 自动 rollback）
   - 返回 `pending_confirm: { expires_at, timeout_seconds, operation }`
   - **响应头附加 `Connection: close`**——告诉浏览器不复用此 TCP 连接。因 apply 后 `pfctl -F states` 会杀死当前连接，浏览器需重新建连发后续请求
10. **如果防火墙未启用**：直接返回（规则加载但不强制，无需倒计时）

### 防锁死机制（anti-lockout）

**目的**：防止用户 apply 或 enable 新规则后，规则阻断了管理连接（HTTP/SSH），导致无法远程恢复。

**机制**：备份 → 应用 → 倒计时确认 → 超时自动回滚。

**数据结构**：`/var/db/fwp/firewall_pending.json`（JSON 文件，原子写入）：
```json
{
  "created_at": 1705312200,
  "expires_at": 1705312260,
  "operation": "apply",
  "driver": "pf",
  "was_enabled": true,
  "backup_config": "# Managed by ...\nblock all\n...",
  "status": "pending"
}
```
> 使用独立 JSON 文件而非 DB 表，因为 pending 数据是临时性的，不需要持久化 schema，且便于排查时直接查看。

**触发条件**：
| 操作 | 防火墙已启用 | 触发倒计时 |
|---|---|---|
| apply | 是 | 是 |
| apply | 否 | 否（规则不强制执行，安全） |
| enable | — | 是 |
| switch | — | 否（结果保持 disabled） |

**回滚流程**（`rollback()` in `firewall_gen.rs`）：
1. 将 `backup_config` 写回配置文件（原子写入）
2. 重新加载配置（ipfw: `ipfw -q /etc/ipfw.rules`；pf: `pfctl -f`）
3. 如果 `was_enabled=false`：禁用防火墙（`disable_firewall`）
4. 删除 `/var/db/fwp/firewall_pending.json`

> 回滚通过进程内 `tokio::spawn(sleep)` 定时器触发，执行本地系统命令（`pfctl`/`sysctl`），不经过网络。即使新规则阻断了所有网络流量，本地命令仍可执行。

**启动安全检查**：`main.rs` 启动时检查 `/var/db/fwp/firewall_pending.json` 是否存在。如果有（说明进程非正常退出，定时器丢失），立即回滚。

**API**：
- `POST /api/firewall/confirm` — 确认变更有效：清除 pending 记录 + staging 写入 DB（全量 replace）+ 删除 staging
- `POST /api/firewall/rollback` — 用户主动回滚：恢复备份配置 + 清除 pending 记录 + 删除 staging（DB 从未改动）
- `POST /api/firewall/discard` — 丢弃未提交的 staging 变更：仅删除 staging 文件（DB 从未改动，无 pending 记录）。与 rollback 的区别：discard 在 apply 之前使用（规则尚未加载到内核），rollback 在 apply 之后使用（规则已加载、需恢复备份）

**前端**：
- apply/enable 返回 `pending_confirm` 时，弹出倒计时对话框（`countdown` 类型）
- 对话框标题/描述/按钮文字根据 `operation` 区分：
  - **apply**（改规则）：标题"确认防火墙变更"，描述"新规则已生效…否则恢复之前的规则"，按钮「恢复规则」/「保持变更」
  - **enable**（启动）：标题"确认防火墙启动"，描述"防火墙已启动…否则停止防火墙"，按钮「停止」/「保持启动」
- 倒计时归零自动触发 rollback
- **定时器清理**（`useCountdown` in `composables/useDialog.js`）：倒计时通过 `setInterval`（500ms）更新对话框的 `secs`/`pct`。必须在对话框关闭时清理定时器，否则幽灵定时器会在归零时调用 `resolveDialog('rollback')` 关闭用户后续打开的任何对话框（`ui.resolveDialog` 操作全局唯一的 `dialog.value`）。双重防护：
  1. `setInterval` 回调开头检查 `ui.dialog !== dialogObj`（对话框已被替换或清空时立即 `clearInterval`）
  2. `promise.then(() => clearInterval(timer))`（promise resolve 时主动清理）
- 确认/恢复后重新加载规则列表（`loadRules()`），显示 DB 真实状态
- 倒计时弹出 1 秒后探测 `/api/firewall/status`，如果不可达则显示"FWP 服务不可达"警告（探测回调同样检查 `ui.dialog !== dialogObj`，避免更新已关闭的对话框）
- 如果 apply 请求因连接中断而失败，前端通过 status 轮询检测 `pending_confirm` 并弹出对话框

### 文件结构

```
src/
├── firewall_gen.rs          # 类型定义、DB CRUD、配置生成、驱动操作
├── handlers/firewall.rs     # API handler 函数
├── db.rs                    # firewall_rules / firewall_state 表创建（migrate 函数内）
└── app.rs                   # 路由注册

frontend/src/
├── pages/
│   ├── FirewallRulesPage.vue    # 规则管理（状态卡片 + 规则列表 + 配置预览弹窗 + 初始化向导）
│   ├── FirewallTablesPage.vue   # IP 名单管理（可折叠列表 + 条目增删）
│   └── FirewallSettingsPage.vue # 设置（引擎切换 + 模式切换 + 启动/停止按钮）
├── lib/menu.js                  # 两级菜单：/firewall → rules/tables/settings
├── composables/useDialog.js     # useCodePreview() + useCountdown() + useFormModal()（含 submitHandler 错误内联展示）
└── components/ui/DialogHost.vue # 通用弹窗（confirm/alert/code/countdown/form）
```

### 前端表单弹窗模式

防火墙规则编辑表单（`FirewallRulesPage.vue` → `makeFields()` → `DialogHost.vue` form 类型）的控件类型与布局约定：

- **Radio（pill 样式）**：动作（allow/deny/reject）、方向（in/out）、源地址类型、目的地址类型 — 使用 `type: 'radio'`，渲染为 `radio-pill-group`（横排胶囊按钮，hover 底色，选中高亮）
- **Select**：协议、ICMP 类型、IP 名单选择 — 使用 `type: 'select'`
- **Checkbox**：记录日志 — 使用 `type: 'checkbox'` + `desc` 属性提供描述文字，渲染为 `confirm-opt` 样式（与确认弹窗选项一致，hover 底色）
- **行布局**：通过 `half: true` + `row: '同值'` 将两个字段放在同一行（`form-row-half` flex 容器）。协议与记录日志共用 `row: 'proto-log'`
- **提交错误处理**：`submitHandler` 模式 — 传入异步函数，API 报错时错误显示在表单弹窗内（`errorMessage` 区域），弹窗保持打开，用户可修正后重试。不会丢失输入内容

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/firewall/status` | 防火墙整体状态 |
| POST | `/api/firewall/initialize` | 初始化 `{ driver, mode }` |
| POST | `/api/firewall/switch` | 切换驱动 `{ driver }` |
| POST | `/api/firewall/enable` | 启用防火墙（同步 rc.conf） |
| POST | `/api/firewall/disable` | 禁用防火墙（同步 rc.conf） |
| PUT | `/api/firewall/mode` | 切换模式 `{ mode }` |
| GET | `/api/firewall/rules` | 列出所有规则 |
| POST | `/api/firewall/rules` | 添加规则 |
| PUT | `/api/firewall/rules/{id}` | 修改规则 |
| DELETE | `/api/firewall/rules/{id}` | 删除规则 |
| PUT | `/api/firewall/rules/{id}/toggle` | 启用/禁用规则 |
| PUT | `/api/firewall/rules/reorder` | 重排序 `{ ordered_ids }` |
| POST | `/api/firewall/apply` | 生成配置 + 加载规则（已启用时触发防锁死倒计时） |
| POST | `/api/firewall/confirm` | 确认变更有效，清除 pending 记录 |
| POST | `/api/firewall/rollback` | 回滚到备份配置 |
| POST | `/api/firewall/discard` | 丢弃未提交的 staging 变更（仅删 staging 文件）|
| GET | `/api/firewall/config` | 预览生成的配置文件内容 |
| GET | `/api/firewall/tables` | 列出所有 IP 名单（含条目） |
| POST | `/api/firewall/tables` | 创建 IP 名单 `{ name, description }` |
| PUT | `/api/firewall/tables/{id}` | 修改名单名称/描述 |
| DELETE | `/api/firewall/tables/{id}` | 删除名单（级联删除条目） |
| POST | `/api/firewall/tables/{id}/entries` | 添加条目 `{ address }` |
| DELETE | `/api/firewall/tables/{id}/entries/{eid}` | 删除条目 |

## 外部依赖

- `/sbin/ipfw` — ipfw 规则管理
- `/sbin/pfctl` — pf 规则管理
- `/usr/sbin/service` — `service pf start`（启用 PF，含 kldload）和 `service pf reload`（应用规则）
- `/sbin/kldload` — 加载内核模块
- `/sbin/kldstat` — 检查模块是否已加载
- `/sbin/sysctl` — 运行时参数设置
- `/usr/sbin/sysrc` — rc.conf 读写（复用 `sysrc.rs` 模块）
- `/sbin/ipfw` — 加载 ipfw 规则文件（`ipfw -q /etc/ipfw.rules`，pathname 模式）+ 语法验证（`ipfw -n -q`）

## 持久化文件

| 文件 | 用途 |
|---|---|
| `/etc/pf.conf` | PF 配置文件（由 fwp 生成，`service pf reload` 加载） |
| `/etc/ipfw.rules` | ipfw 规则文件（由 fwp 生成，`ipfw -q /etc/ipfw.rules` 加载） |
| `/var/db/fwp/firewall_staging.json` | 暂存文件——防火墙已启用时，规则修改的快照（全量规则+表）。confirm 时提交到 DB，rollback 时删除 |
| `/var/db/fwp/firewall_pending.json` | 防锁死 pending 记录——apply/enable 时的备份配置+超时信息 |

## 配置项

无专用 `fwp.toml` 配置项。防火墙状态存储在 SQLite `firewall_state` 表中。

## 已知限制 / TODO

1. **NAT/转发规则已实现** — 详见 [28-nat.md](28-nat.md)，支持 SNAT/DNAT 独立模型、嵌入式生成、复用 staging/apply/防锁死链路。BINAT 完整支持为 P2
2. **不导入已有配置** — 初始化时生成空白规则集（P2-F 计划）
3. **pf 的 `(self)` 地址** — 表示本机所有 IP，不支持指定接口地址
4. **无规则可达性分析** — 不检测规则冲突或冗余（P2-B 计划）
5. **ipfw 的 `default_to_accept` boot tunable** — 不修改 loader.conf，通过规则 65534 在运行时控制
6. **删除被规则引用的名单** — 不检查引用关系，删除后 pf apply 会因 table 未定义而失败（ipfw 则该 table 匹配空集）（P2-B 计划引用完整性检查）
7. **ipfw table 持久化** — 依赖规则脚本重建，不使用 `ipfw table ... add` 的持久化机制
8. **无配置备份/版本历史** — 当前防锁死机制仅保留最近一次备份，不支持历史版本浏览（P2-D 计划）
9. **模式切换不触发倒计时** — `set_mode` 直接重新加载规则（pf）或切换默认策略（ipfw），不走防锁死流程
10. **`position` 字段类型** — `next_position()` 使用 `COALESCE(MAX(position), -1) + 1` 在 `i64` 中计算后转 `u32`，避免空表时 `-1 as u32` 溢出
