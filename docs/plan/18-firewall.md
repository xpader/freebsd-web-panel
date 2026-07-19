# 模块设计：防火墙（双驱动 ipfw / pf）

> 原 `40-system.md §5` 的简要 pf 规划，本文档为展开的详细设计方案。
> 与原方案的核心区别：**双驱动架构**（ipfw + pf 可选可切换）、**结构化规则 CRUD**、**黑白名单模式**、**rc.conf 初始化管理**。
>
> 实现已完成，详见 [docs/impl/27-firewall.md](../impl/27-firewall.md)。

## 1. 目标

在 Web 面板中提供统一的防火墙管理界面，支持 FreeBSD 两大防火墙引擎：

| 能力 | 说明 |
|---|---|
| **驱动选择** | 首次使用必须选择 ipfw 或 pf 作为活动驱动 |
| **驱动切换** | 可随时切换驱动，旧驱动禁用、新驱动初始化，规则统一存储无需迁移 |
| **规则 CRUD** | 以结构化表单（动作/协议/源/目/端口/接口/ICMP类型）增删改规则，面板自动生成对应语法的配置 |
| **黑白名单模式** | 白名单=默认拒绝（仅放行匹配规则），黑名单=默认放行（仅阻断匹配规则） |
| **rc.conf 初始化** | 初始化和切换时自动写入/清除对应 rc.conf 条目，确保重启后状态一致 |
| **统一规则存储** | 规则与引擎解耦——一套规则在 DB 中，切换引擎时自动生成对应语法配置文件 |

分两期实现：

| 期 | 范围 |
|---|---|
| **P1 核心功能** | 驱动选择/切换、模式切换、规则增删改、rc.conf 初始化、文件生成与加载 |
| **P2 增强** | 规则拖拽排序、批量操作、NAT/转发规则、pf 表管理、规则导入（解析已有配置文件） |

---

## 2. 整体架构

### 2.1 为什么需要双驱动？

FreeBSD 有两大防火墙引擎，各有适用场景：

| 特性 | ipfw | pf |
|---|---|---|
| 语法风格 | 命令式（`add N allow ...`） | 声明式（`pass in ...`） |
| 规则编号 | 有编号（1-65535），按编号顺序匹配 | 无编号，按文件顺序匹配 |
| 默认策略 | sysctl `net.inet.ip.fw.default_to_accept` 控制 | 规则文件中 `block all` / 默认 pass |
| 状态保持 | `keep-state` / `check-state` | 默认 `keep state`（自动） |
| 匹配语义 | 首条匹配生效（first-match） | 最后匹配生效（last-match，除非 `quick`） |
| 模块加载 | `ipfw.ko`（或编译进内核） | `pf.ko`（或编译进内核） |
| 典型用途 | 流量整形、dummynet、NAT | 状态过滤、反欺骗、OpenBSD 同源语法 |

面板不强制选择某一个——用户根据需求选择，且可随时切换。

### 2.2 驱动选择与切换流程

```
┌─────────────────────────────────────────────────────────────┐
│                    防火墙状态机                              │
│                                                             │
│  Uninitialized ──initialize(driver, mode)──► Active(ipfw)  │
│       │                                              │      │
│       │                                  switch(pf)  │      │
│       └──initialize(driver, mode)──► Active(pf) ◄────┘      │
│                                            │                │
│                                   switch(ipfw)               │
│                                            ▼                │
│                                      Active(ipfw)            │
│                                                             │
│  任何 Active 状态下：                                        │
│    enable/disable ──► 运行时启停（不改 boot 配置）            │
│    change_mode    ──► 切换黑白名单                            │
│    switch(driver) ──► 切换驱动（保留旧规则，加载新规则）       │
└─────────────────────────────────────────────────────────────┘
```

**初始化流程（以 ipfw 为例）：**

1. 写入 rc.conf（通过 sysrc）：
   - `firewall_enable="YES"`
   - `firewall_script="/etc/ipfw.rules"`
   - `firewall_logging="YES"`
2. 清除冲突项：删除 `firewall_type`（避免与 `firewall_script` 冲突）
3. 写入 sysctl.conf（黑白名单模式持久化）：
   - 白名单：`net.inet.ip.fw.default_to_accept=0`
   - 黑名单：`net.inet.ip.fw.default_to_accept=1`
4. 加载内核模块：`kldload ipfw`（如未编译进内核）
5. 设置运行时 sysctl：`sysctl net.inet.ip.fw.default_to_accept=0|1`
6. 生成规则文件：从数据库生成 `/etc/ipfw.rules`
7. 加载规则：执行 `/etc/ipfw.rules`（即 `sh /etc/ipfw.rules`）
8. 启用防火墙：`sysctl net.inet.ip.fw.enable=1`

**初始化流程（以 pf 为例）：**

1. 写入 rc.conf：
   - `pf_enable="YES"`
   - `pf_rules="/etc/pf.conf"`
2. 加载内核模块：`kldload pf`（如未编译进内核）
3. 生成规则文件：从数据库生成 `/etc/pf.conf`（含黑白名单默认规则）
4. 加载规则：`pfctl -f /etc/pf.conf`
5. 启用 pf：`pfctl -e`

**切换流程（ipfw → pf）：**

1. 禁用 ipfw 运行时：`sysctl net.inet.ip.fw.enable=0`
2. 写入 rc.conf：`firewall_enable="NO"`
3. 加载 pf 内核模块：`kldload pf`
4. 从数据库生成 `/etc/pf.conf`（pf 规则 + 当前模式）
5. 加载规则：`pfctl -f /etc/pf.conf`
6. 启用 pf：`pfctl -e`
7. 写入 rc.conf：`pf_enable="YES"`
8. 更新数据库：`active_driver=pf`

**切换流程（pf → ipfw）：** 对称操作，不再赘述。

### 2.3 规则管理模型

面板使用**结构化规则抽象**——用统一的 Rust 结构体表示规则语义，生成时按驱动转换为对应语法。

```
┌─────────────┐         ┌──────────────┐
│  前端表单    │ ──►    │  FirewallRule │  (结构化，存 SQLite)
│ (JSON API)  │         │  (Rust struct) │
└─────────────┘         └──────┬───────┘
                               │
                    ┌──────────┴──────────┐
                    │                     │
               ipfw 生成器              pf 生成器
                    │                     │
                    ▼                     ▼
          /etc/ipfw.rules           /etc/pf.conf
          (shell 脚本)              (pf 规则文件)
```

**为什么用结构化抽象而非直接编辑配置文件？**

- ipfw 和 pf 语法差异大，直接编辑需要用户熟悉对应语法
- 结构化表单让用户无需了解语法细节即可管理规则
- 面板自动处理编号分配（ipfw）、quick 关键字（pf）、状态保持等细节
- 两套规则可共存于数据库，切换驱动不丢失

**规则存储策略：SQLite 为主，配置文件为生成产物**

- 规则的结构化数据存储在 SQLite（`firewall_rules` 表）
- 配置文件（`/etc/ipfw.rules`、`/etc/pf.conf`）在「应用」时从数据库重新生成
- 数据库是 single source of truth；配置文件是派生产物
- 首次初始化时可选导入已有配置文件中的规则（P2）

### 2.4 黑白名单模式

| 模式 | 含义 | ipfw 实现 | pf 实现 |
|---|---|---|---|
| **白名单** | 默认拒绝所有流量，仅放行匹配 `allow` 规则的流量 | `net.inet.ip.fw.default_to_accept=0`（规则 65535 = deny all） | 配置文件首行 `block all`，用户规则为 `pass` |
| **黑名单** | 默认放行所有流量，仅阻断匹配 `deny` 规则的流量 | `net.inet.ip.fw.default_to_accept=1`（规则 65535 = allow all） | 无默认阻断规则（pf 默认 pass），用户规则为 `block` |

**ipfw 模式切换：**
- 仅需修改 sysctl（运行时立即生效）+ sysctl.conf（持久化）
- 无需重新加载规则文件

**pf 模式切换：**
- 需重新生成配置文件（增/删 `block all` 首行）
- 执行 `pfctl -f /etc/pf.conf` 重新加载（原子操作，极短中断）

**模式切换的 UX 考量：**
- 白名单→黑名单：用户已有的 `allow` 规则变成「在默认放行基础上的额外放行」，实际效果不变但仍保留
- 黑名单→白名单：用户已有的 `deny` 规则变成「在默认拒绝基础上的额外拒绝」，但之前放行的流量可能被阻断
- **面板必须在切换模式时弹出警告**，说明规则语义变化和潜在影响

---

## 3. 数据模型

### 3.1 Rust 结构体

```rust
// ── 枚举 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FirewallDriver {
    Ipfw,
    Pf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FirewallMode {
    Whitelist,  // 默认拒绝
    Blacklist,  // 默认放行
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleAction {
    Allow,
    Deny,
    Reject,     // deny + 回复 RST/ICMP
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleDirection {
    In,         // 入站
    Out,        // 出站
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleProtocol {
    Tcp,
    Udp,
    Icmp,
    Icmpv6,
    Any,
}

/// 地址规格——源和目的共用。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AddressSpec {
    kind: AddressKind,
    value: String,          // kind=Any 时为空；kind=Single 时为 "192.168.1.1"；kind=Cidr 时为 "10.0.0.0/24"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AddressKind {
    Any,        // 任意地址
    Single,     // 单个 IP（v4 或 v6）
    Cidr,       // CIDR 网段
    Me,         // 本机所有 IP（ipfw: "me"；pf: "(egress)" 或 "(self)"）
}

/// 端口规格（仅 TCP/UDP 有效）。
/// 支持："80" | "80,443" | "1024-65535"
type PortSpec = String;

// ── 核心结构 ──

/// 结构化防火墙规则——两套驱动共用的抽象表示。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FirewallRule {
    id: i64,                        // DB 主键（前端不直接使用，用 position 排序）
    driver: FirewallDriver,         // 所属驱动
    position: u32,                  // 排序位置（0-based，按此值升序排列）
    enabled: bool,

    // 规则语义
    action: RuleAction,
    direction: RuleDirection,
    protocol: RuleProtocol,
    source: AddressSpec,
    source_port: Option<PortSpec>,
    destination: AddressSpec,
    destination_port: Option<PortSpec>,
    interface: Option<String>,      // 绑定接口（如 "em0"），None = 所有接口
    log: bool,                      // 是否记录匹配日志

    // 元数据
    description: Option<String>,
    created_at: i64,                // Unix timestamp
    updated_at: i64,
}

/// 防火墙整体状态。
#[derive(Debug, Clone, Serialize)]
struct FirewallStatus {
    driver: Option<FirewallDriver>,     // 当前活动驱动（None = 未初始化）
    initialized: bool,                  // 是否已初始化（DB 中有 active_driver 记录）
    enabled: bool,                      // 防火墙是否在运行
    mode: Option<FirewallMode>,         // 当前黑白名单模式
    module_loaded: bool,                // 内核模块是否已加载
    rules_count: usize,                 // 当前驱动的规则数量（仅 enabled 的）
    pending_apply: bool,                // 是否有未应用的规则变更
}

/// 防火墙配置（存储在 DB 的 firewall_state 表）。
#[derive(Debug, Clone)]
struct FirewallState {
    active_driver: Option<FirewallDriver>,
    mode: FirewallMode,
}
```

### 3.2 SQLite 表结构

```sql
-- 防火墙规则（两套驱动的规则共存于此表）
CREATE TABLE firewall_rules (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    driver      TEXT    NOT NULL,               -- 'ipfw' | 'pf'
    position    INTEGER NOT NULL DEFAULT 0,     -- 排序
    enabled     INTEGER NOT NULL DEFAULT 1,     -- 0=禁用 1=启用
    action      TEXT    NOT NULL,               -- 'allow' | 'deny' | 'reject'
    direction   TEXT    NOT NULL,               -- 'in' | 'out'
    protocol    TEXT    NOT NULL,               -- 'tcp' | 'udp' | 'icmp' | 'icmpv6' | 'any'
    src_kind    TEXT    NOT NULL,               -- 'any' | 'single' | 'cidr' | 'me'
    src_value   TEXT    NOT NULL DEFAULT '',
    src_port    TEXT,                           -- NULL 或端口规格
    dst_kind    TEXT    NOT NULL,
    dst_value   TEXT    NOT NULL DEFAULT '',
    dst_port    TEXT,
    interface   TEXT,                           -- NULL = 所有接口
    log         INTEGER NOT NULL DEFAULT 0,
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX idx_firewall_rules_driver ON firewall_rules(driver);
CREATE INDEX idx_firewall_rules_position ON firewall_rules(driver, position);

-- 面板的防火墙状态（key-value）
CREATE TABLE firewall_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- 记录：
--   'active_driver' → 'ipfw' | 'pf' | (不存在 = 未初始化)
--   'mode'          → 'whitelist' | 'blacklist'
--   'rules_dirty'   → '1' | '0'（是否有未应用的变更）
```

---

## 4. 后端实现

### 4.1 模块结构

```
src/
├── handlers/
│   ├── firewall.rs       # 防火墙 handler 函数 + 数据结构
│   └── mod.rs            # 注册 pub mod firewall;
├── firewall_gen.rs       # 规则生成器（ipfw / pf 配置文件生成）
├── app.rs                # 路由注册
└── db.rs                 # firewall_rules / firewall_state 表创建与访问函数
```

**为什么不把生成器放在 handler 里？** 规则生成逻辑较复杂（~200 行/驱动），独立模块便于测试和复用。

### 4.2 驱动管理（初始化 / 切换 / 启停）

所有操作在 `spawn_blocking` 中执行（涉及多个系统命令和文件 I/O）。

**初始化 handler（`POST /api/firewall/initialize`）：**

```
initialize(driver, mode):
    1. 检查是否已初始化 → 拒绝重复初始化
    2. if driver == Ipfw:
         sysrc firewall_enable="YES"
         sysrc firewall_script="/etc/ipfw.rules"
         sysrc firewall_logging="YES"
         sysrc -x firewall_type                  # 清除冲突项
         写 sysctl.conf: net.inet.ip.fw.default_to_accept=0|1
         kldload ipfw                             # 加载模块
         sysctl net.inet.ip.fw.default_to_accept=0|1   # 运行时
         生成 /etc/ipfw.rules（从 DB + 模式默认规则）
         sh /etc/ipfw.rules                       # 加载规则
         sysctl net.inet.ip.fw.enable=1           # 启用
    3. if driver == Pf:
         sysrc pf_enable="YES"
         sysrc pf_rules="/etc/pf.conf"
         kldload pf                               # 加载模块
         生成 /etc/pf.conf（从 DB + 模式默认规则）
         pfctl -f /etc/pf.conf                    # 加载规则
         pfctl -e                                 # 启用
    4. DB: firewall_state 写入 active_driver=driver, mode=mode
    5. 审计日志
```

**切换 handler（`POST /api/firewall/switch`）：**

```
switch(new_driver):
    1. 读取当前 active_driver
    2. if current == Ipfw:
         sysctl net.inet.ip.fw.enable=0           # 停止 ipfw
         sysrc firewall_enable="NO"               # 取消开机启动
    3. if current == Pf:
         pfctl -d                                 # 停止 pf
         sysrc pf_enable="NO"                     # 取消开机启动
    4. 加载新驱动（同 initialize 步骤 2/3）
    5. DB: firewall_state 更新 active_driver=new_driver
    6. 审计日志
```

**启停 handler：**

```
enable():
    if driver == Ipfw: sysctl net.inet.ip.fw.enable=1
    if driver == Pf:   pfctl -e

disable():
    if driver == Ipfw: sysctl net.inet.ip.fw.enable=0
    if driver == Pf:   pfctl -d
```

> 启停**不修改 rc.conf**（不影响开机行为），仅修改运行时状态。

### 4.3 ipfw 规则生成

**文件格式（`/etc/ipfw.rules`）：**

```sh
#!/bin/sh
# ============================================================
# Managed by FreeBSD Web Panel (fwp) — DO NOT EDIT MANUALLY
# Driver: ipfw | Mode: whitelist (default deny)
# Generated: 2024-01-15T10:30:00Z
# ============================================================

ipfw -q flush                                  # 清除所有规则（保留默认 65535）

# ---- Managed Rules ----

# [00100] Allow HTTP
ipfw -q add 00100 allow tcp from any to any dst-port 80 in

# [00200] Allow HTTPS
ipfw -q add 00200 allow tcp from any to any dst-port 443 in keep-state

# [00300] Deny SSH from untrusted
ipfw -q add 00300 deny log tcp from 10.0.0.0/8 to me dst-port 22 in

# ---- End Managed Rules ----
```

**规则编号方案：**

- 面板管理的规则编号从 `00100` 开始，步进 `100`
- `position=0` → 规则号 `00100`，`position=1` → `00200`，以此类推
- 编号在生成时自动计算，用户不需要关心
- 保留 `00001-00099` 给未来可能的系统级优先规则
- 默认规则 `65535` 由 sysctl 控制（黑白名单模式），不在文件中显式添加

**ipfw 规则生成器伪代码：**

```rust
fn generate_ipfw(rules: &[FirewallRule], mode: FirewallMode) -> String {
    let mut buf = header("ipfw", mode);

    // flush
    buf.push_str("ipfw -q flush\n\n");

    for (i, rule) in rules.iter().filter(|r| r.enabled).enumerate() {
        let number = (i + 1) * 100;  // 00100, 00200, ...

        // 动作
        let action = match rule.action {
            Allow => "allow",
            Deny  => "deny",
            Reject => "reject",
        };

        // 地址
        let src = format_address_ipfw(&rule.source);
        let dst = format_address_ipfw(&rule.destination);

        // 协议
        let proto = match rule.protocol {
            Any => "ip",
            Tcp => "tcp",
            Udp => "udp",
            Icmp => "icmp",
            Icmpv6 => "ipv6-icmp",
        };

        // 方向
        let dir = match rule.direction {
            In => "in",
            Out => "out",
        };

        // 端口
        let src_port = rule.source_port.as_ref()
            .map(|p| format!(" {}", p)).unwrap_or_default();
        let dst_port = rule.destination_port.as_ref()
            .map(|p| format!(" dst-port {}", p)).unwrap_or_default();

        // 接口
        let iface = rule.interface.as_ref()
            .map(|ifn| format!(" {}", if_ipfw_dir(rule.direction, ifn)))
            .unwrap_or_default();

        // 日志
        let log = if rule.log { " log" } else { "" };

        // 状态保持（仅 allow 规则）
        let state = if matches!(rule.action, Allow) { " keep-state" } else { "" };

        buf.push_str(&format!(
            "# [{:05}] {}\nipfw -q add {:05}{} {} {} from {}{} to {}{}{} {}{}\n\n",
            number, rule.description.as_deref().unwrap_or(""),
            number, log, action, proto, src, src_port, dst, dst_port, dir, iface, state
        ));
    }

    buf
}
```

**ipfw 地址格式映射：**

| AddressKind | ipfw 语法 |
|---|---|
| Any | `any` |
| Single | `192.168.1.1` |
| Cidr | `10.0.0.0/24` |
| Me | `me` |

**ipfw 方向-接口映射：**

| Direction | 语法 |
|---|---|
| In + interface `em0` | `in recv em0` |
| Out + interface `em0` | `out xmit em0` |

### 4.4 pf 规则生成

**文件格式（`/etc/pf.conf`，白名单模式）：**

```pf
# ============================================================
# Managed by FreeBSD Web Panel (fwp) — DO NOT EDIT MANUALLY
# Driver: pf | Mode: whitelist (default deny)
# Generated: 2024-01-15T10:30:00Z
# ============================================================

# 默认策略：阻断所有流量（白名单模式）
block all

# ---- Managed Rules ----

# Allow HTTP
pass quick in inet proto tcp from any to any port 80 flags S/SA keep state

# Allow HTTPS
pass quick in inet proto tcp from any to any port 443 flags S/SA keep state

# Deny SSH from untrusted
block quick in inet proto tcp from 10.0.0.0/8 to any port 22

# ---- End Managed Rules ----
```

**文件格式（黑名单模式）：**

```pf
# ============================================================
# Managed by FreeBSD Web Panel (fwp) — DO NOT EDIT MANUALLY
# Driver: pf | Mode: blacklist (default allow)
# Generated: 2024-01-15T10:30:00Z
# ============================================================

# 默认策略：放行所有流量（黑名单模式）
# pf 无匹配规则时默认 pass，无需显式规则

# ---- Managed Rules ----

# Block SSH from untrusted
block quick in inet proto tcp from 10.0.0.0/8 to any port 22

# ---- End Managed Rules ----
```

**pf 规则生成的关键决策：**

1. **所有用户规则使用 `quick`**：使匹配语义为「首个匹配生效」（first-match），与 ipfw 保持一致，避免 pf 默认的 last-match 语义带来的混乱
2. **白名单的 `block all` 不带 `quick`**：这样后续的 `pass quick` 规则才能覆盖它
3. **地址族自动判断**：根据地址格式（IPv4/IPv6）自动选择 `inet` / `inet6`；Any 时用 `inet`
4. **`pass` 规则自动加 `keep state`**：保持状态连接（pf 现代 版本默认 `keep state`，显式写出更清晰）

**pf 地址格式映射：**

| AddressKind | pf 语法 |
|---|---|
| Any | `any` |
| Single | `192.168.1.1` |
| Cidr | `10.0.0.0/24` |
| Me | `(self)` |

**pf 动作映射：**

| RuleAction | pf 语法 |
|---|---|
| Allow | `pass quick` |
| Deny | `block quick` |
| Reject | `block quick return` |

### 4.5 黑白名单模式管理

```
change_mode(new_mode):
    1. 更新 DB firewall_state mode=new_mode
    2. if driver == Ipfw:
         sysctl net.inet.ip.fw.default_to_accept = if whitelist { 0 } else { 1 }
         更新 /etc/sysctl.conf 中的对应行（原子替换）
         标记 rules_dirty = 0（模式切换不需要重新加载 ipfw 规则）
    3. if driver == Pf:
         重新生成 /etc/pf.conf（增/删 block all 首行）
         pfctl -f /etc/pf.conf
         标记 rules_dirty = 0
    4. 审计日志
```

**sysctl.conf 编辑策略：**

不使用 `sysctl(8)` 命令写入 sysctl.conf（sysctl 命令只改运行时值）。而是：
1. 读取 `/etc/sysctl.conf` 全文
2. 用正则替换/增删 `net.inet.ip.fw.default_to_accept=...` 行
3. 原子写入（写到临时文件 → rename）
4. 同时 `sysctl` 命令设置运行时值

### 4.6 规则 CRUD 与应用

**CRUD 操作仅修改数据库，不立即应用：**

```
POST   /api/firewall/rules     → 插入 DB，设 rules_dirty=1
PUT    /api/firewall/rules/{id} → 更新 DB，设 rules_dirty=1
DELETE /api/firewall/rules/{id} → 从 DB 删除，设 rules_dirty=1
PUT    /api/firewall/rules/reorder → 批量更新 position，设 rules_dirty=1
```

**应用操作（`POST /api/firewall/apply`）：**

```
apply():
    1. 从 DB 读取当前驱动的所有 enabled 规则，按 position 排序
    2. 调用生成器，生成配置文件内容
    3. 校验语法：
         ipfw: 无直接 dry-run；用 `ipfw -n -q add ...` 逐条检查（-n = 不实际添加）
         pf:   `pfctl -n -f /tmp/generated.conf`（整个文件校验）
    4. 校验失败 → 返回错误，不写入文件
    5. 校验通过 → 原子写入配置文件（临时文件 → rename）
    6. 加载规则：
         ipfw: `sh /etc/ipfw.rules`
         pf:   `pfctl -f /etc/pf.conf`
    7. 设 rules_dirty=0
    8. 审计日志
```

### 4.7 状态查询

```
get_status():
    1. 从 DB 读取 active_driver、mode
    2. 判断 module_loaded：
         ipfw: `kldstat -q -n ipfw` 或 sysctl net.inet.ip.fw.enable 是否存在
         pf:   `kldstat -q -n pf` 或 sysctl net.pf.enabled 是否存在
    3. 判断 enabled：
         ipfw: sysctl -n net.inet.ip.fw.enable（1=enabled, 0=disabled）
         pf:   pfctl -s info | grep "Status"（"Enabled" / "Disabled"）
    4. 统计 rules_count（DB 中当前驱动的 enabled 规则数）
    5. 读取 rules_dirty
```

### 4.8 输入校验

| 字段 | 校验规则 |
|---|---|
| `interface` | 匹配 `^[a-zA-Z0-9_.]+$`，1-15 字符（与 network 模块一致），或 null |
| `source.value` / `destination.value` | kind=Single 时校验合法 IPv4/IPv6；kind=Cidr 时校验 CIDR 格式 |
| `source_port` / `destination_port` | 匹配 `^(\d+)(-(\d+))?(,(\d+)(-(\d+))?)*$`，端口范围 1-65535 |
| `description` | 长度 ≤ 200，无换行 |

---

## 5. API

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/api/firewall/status` | 防火墙整体状态（驱动/模式/运行/规则数/待应用） | 是 |
| POST | `/api/firewall/initialize` | 初始化防火墙 `{ driver, mode }` | 是 |
| POST | `/api/firewall/switch` | 切换驱动 `{ driver }` | 是 |
| POST | `/api/firewall/enable` | 运行时启用防火墙 | 是 |
| POST | `/api/firewall/disable` | 运行时禁用防火墙 | 是 |
| PUT | `/api/firewall/mode` | 切换黑白名单 `{ mode }` | 是 |
| GET | `/api/firewall/rules` | 列出当前驱动规则 | 是 |
| POST | `/api/firewall/rules` | 添加规则（body = FirewallRule，不含 id/timestamps） | 是 |
| PUT | `/api/firewall/rules/{id}` | 修改规则 | 是 |
| DELETE | `/api/firewall/rules/{id}` | 删除规则 | 是 |
| PUT | `/api/firewall/rules/reorder` | 重排序 `{ ordered_ids: [3, 1, 5, 2] }` | 是 |
| POST | `/api/firewall/apply` | 应用变更（生成配置 + 校验 + 加载） | 是 |
| GET | `/api/firewall/config` | 预览生成的配置文件内容 | 是 |

### API 请求/响应示例

**POST /api/firewall/initialize**
```json
// Request
{ "driver": "ipfw", "mode": "whitelist" }

// Response 200
{ "driver": "ipfw", "mode": "whitelist", "initialized": true }
```

**POST /api/firewall/rules**
```json
// Request
{
    "action": "allow",
    "direction": "in",
    "protocol": "tcp",
    "source": { "kind": "any", "value": "" },
    "destination": { "kind": "me", "value": "" },
    "destination_port": "80",
    "interface": "em0",
    "log": false,
    "description": "Allow HTTP"
}

// Response 201
{
    "id": 1,
    "driver": "ipfw",
    "position": 0,
    "enabled": true,
    "action": "allow",
    "direction": "in",
    "protocol": "tcp",
    "source": { "kind": "any", "value": "" },
    "source_port": null,
    "destination": { "kind": "me", "value": "" },
    "destination_port": "80",
    "interface": "em0",
    "log": false,
    "description": "Allow HTTP",
    "created_at": 1705312200,
    "updated_at": 1705312200
}
```

**GET /api/firewall/status**
```json
{
    "driver": "ipfw",
    "initialized": true,
    "enabled": true,
    "mode": "whitelist",
    "module_loaded": true,
    "rules_count": 3,
    "pending_apply": false
}
```

**GET /api/firewall/config**
```json
{
    "driver": "ipfw",
    "content": "#!/bin/sh\n# Managed by fwp ...\nipfw -q flush\n\nipfw -q add 00100 ...\n"
}
```

---

## 6. rc.conf 初始化项

### 6.1 ipfw 驱动

| rc.conf key | 值 | 说明 |
|---|---|---|
| `firewall_enable` | `"YES"` | 开机启动 ipfw 服务 |
| `firewall_script` | `"/etc/ipfw.rules"` | 规则脚本路径（面板管理） |
| `firewall_logging` | `"YES"` | 启用日志 |
| ~~`firewall_type`~~ | （删除） | 初始化时清除，避免与 `firewall_script` 冲突 |

sysctl.conf：

| key | 白名单值 | 黑名单值 |
|---|---|---|
| `net.inet.ip.fw.default_to_accept` | `0` | `1` |

### 6.2 pf 驱动

| rc.conf key | 值 | 说明 |
|---|---|---|
| `pf_enable` | `"YES"` | 开机启动 pf 服务 |
| `pf_rules` | `"/etc/pf.conf"` | 规则文件路径（面板管理） |

> pf 的黑白名单模式不依赖 sysctl，而是通过配置文件中的 `block all` 规则实现。

### 6.3 清理（切换驱动时禁用旧驱动）

| 操作 | ipfw → pf 时 | pf → ipfw 时 |
|---|---|---|
| 旧驱动 rc.conf | `firewall_enable="NO"` | `pf_enable="NO"` |
| 旧驱动运行时 | `sysctl net.inet.ip.fw.enable=0` | `pfctl -d` |

> **注意**：不删除旧驱动的 `firewall_script`/`pf_rules` 配置项（保留路径以便切回时复用），仅设置 `*_enable="NO"`。

---

## 7. 前端设计

### 7.1 页面布局

`frontend/src/pages/FirewallPage.vue` 替换现有 `PfPage.vue` 占位组件。

菜单项更新：`nav.pf` → `nav.firewall`（或保持 `nav.pf` 但指向新页面）。

**页面结构：**

```
┌─────────────────────────────────────────────────────────┐
│  防火墙                              [刷新]              │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  ┌─ 状态卡片 ────────────────────────────────────────┐  │
│  │  驱动: [ipfw ▼]  模式: [白名单 ▼]  状态: ● 运行中 │  │
│  │  内核模块: 已加载    规则数: 3    待应用: 否       │  │
│  │  [启用/禁用]  [切换驱动]  [切换模式]              │  │
│  └───────────────────────────────────────────────────┘  │
│                                                         │
│  ┌─ 规则列表 ────────────────────────────────────────┐  │
│  │  [+ 添加规则]                      [应用变更]     │  │
│  │                                                    │  │
│  │  #  动作  方向  协议  源          目的       端口  │  │
│  │  ─  ────  ────  ────  ──────────  ──────────  ──── │  │
│  │  1  ✓允许 入站  TCP   any        me         80    │  │
│  │  2  ✓允许 入站  TCP   any        me         443   │  │
│  │  3  ✗拒绝 入站  TCP   10.0.0.0/8  me         22    │  │
│  │                                                    │  │
│  │  [编辑] [删除] [启用/禁用] [上移/下移]            │  │
│  └───────────────────────────────────────────────────┘  │
│                                                         │
│  ┌─ 配置预览 ────────────────────────────────────────┐  │
│  │  #!/bin/sh                                         │  │
│  │  # Managed by fwp ...                              │  │
│  │  ipfw -q flush                                     │  │
│  │  ipfw -q add 00100 allow tcp from any ...         │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### 7.2 未初始化状态

首次进入防火墙页面时，显示初始化向导（替代状态卡片 + 规则列表）：

```
┌─────────────────────────────────────────────────────────┐
│                   防火墙初始化                            │
│                                                         │
│  本面板支持 ipfw 和 pf 两种防火墙引擎。                   │
│  请选择要使用的引擎：                                     │
│                                                         │
│  ┌─────────────────┐    ┌─────────────────┐            │
│  │   ◉ ipfw        │    │   ○ pf          │            │
│  │   命令式语法     │    │   声明式语法     │            │
│  │   按编号匹配     │    │   按 quick 匹配  │            │
│  │   支持 dummynet │    │   原生状态保持   │            │
│  └─────────────────┘    └─────────────────┘            │
│                                                         │
│  默认策略：                                              │
│   ◉ 白名单（默认拒绝，仅放行匹配规则）                    │
│   ○ 黑名单（默认放行，仅阻断匹配规则）                    │
│                                                         │
│              [取消]              [初始化]                 │
└─────────────────────────────────────────────────────────┘
```

### 7.3 规则编辑对话框

使用 `useFormModal` composable 弹出模态表单：

```
┌─────────────────────────────────────────────────────────┐
│  添加规则                                         [×]    │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  描述:   [Allow HTTP traffic                          ]  │
│                                                         │
│  动作:   [✓允许 ▼]    方向: [入站 ▼]    日志: [○]      │
│                                                         │
│  协议:   [TCP ▼]                                        │
│                                                         │
│  源地址: [任意 ▼]                                       │
│  源端口: [                              ] (仅TCP/UDP)   │
│                                                         │
│  目的地址: [本机 ▼]                                     │
│  目的端口: [80                           ] (仅TCP/UDP)  │
│                                                         │
│  接口:   [em0 ▼] (可选)                                 │
│                                                         │
│                              [取消]    [保存]            │
└─────────────────────────────────────────────────────────┘
```

### 7.4 切换驱动确认

```
┌─────────────────────────────────────────────────────────┐
│  ⚠ 切换防火墙驱动                                       │
│                                                         │
│  确定要从 ipfw 切换到 pf 吗？                            │
│                                                         │
│  • ipfw 将被停止并禁用开机启动                           │
│  • pf 将被加载并启用                                     │
│  • ipfw 的规则将保留在面板中（切回时可恢复）              │
│  • pf 的规则将立即生效                                   │
│  • 切换过程中可能有短暂的网络中断                         │
│                                                         │
│                              [取消]    [确认切换]        │
└─────────────────────────────────────────────────────────┘
```

### 7.5 消息反馈策略

遵循项目约定（成功 → toast，失败 → 弹窗）：

| 场景 | 方式 | 说明 |
|---|---|---|
| 规则添加/编辑/删除成功 | `useToast()` | 用户漏看不影响（已入库） |
| 应用变更成功 | `useToast()` | "规则已应用" |
| 应用变更失败（语法校验失败） | `useAlert()` | **必须弹窗**——用户需看到失败原因 |
| 切换驱动成功 | `useToast()` | "已切换到 pf" |
| 切换驱动失败 | `useAlert()` | **必须弹窗**——可能导致防火墙状态不一致 |
| 模式切换需警告 | `useConfirm()` | 二次确认，说明规则语义变化 |
| 初始化成功 | `useToast()` | "防火墙已初始化" |
| 初始化失败 | `useAlert()` | **必须弹窗**——用户需知道原因 |

---

## 8. 安全考量

### 8.1 防自锁

用户可能意外添加一条阻断自己 SSH 会话的规则。缓解措施：

1. **默认 allow 规则加 `keep-state`**（ipfw）/ `keep state`（pf）——已建立的连接不受新规则影响
2. **白名单模式初始化时，自动添加一条「放行 SSH」规则**：
   - 面板在初始化时检测当前 SSH 连接（`$SSH_CONNECTION` 环境变量或 `sockstat`），自动添加一条放行规则
3. **应用前校验**：如果生成的规则集会导致面板自身的连接被阻断，弹出警告（P2 增强，需分析规则集可达性）
4. **`disable` 快捷方式**：面板始终提供一键禁用防火墙的按钮（紧急解锁）

### 8.2 文件原子写入

生成配置文件时使用原子写入：
```rust
// 写到临时文件，然后 rename
let tmp = format!("{}.tmp", path);
fs::write(&tmp, &content)?;
fs::rename(&tmp, path)?;
```

### 8.3 配置文件保护

- 配置文件头部有 `# Managed by fwp — DO NOT EDIT MANUALLY` 标记
- `GET /api/firewall/config` 可查看当前生成的配置内容
- 面板不做配置文件「双向同步」——如果用户手动修改了配置文件，下次 apply 会覆盖

### 8.4 sysctl.conf 编辑安全

编辑 `/etc/sysctl.conf` 时不使用 shell 命令拼接，而是：
1. 读取全文到内存
2. 用正则定位 `net.inet.ip.fw.default_to_accept=...` 行
3. 替换或增删
4. 原子写入

---

## 9. 命令参考

### 9.1 ipfw 命令

| 操作 | 命令 |
|---|---|
| 列出规则 | `ipfw -a -d -t list` |
| 添加规则 | `ipfw add NNNN action ...` |
| 删除规则 | `ipfw delete NNNN` |
| 清除所有规则 | `ipfw -q flush` |
| 语法检查 | `ipfw -n -q add NNNN action ...`（-n = 不实际添加） |
| 启用 | `sysctl net.inet.ip.fw.enable=1` |
| 禁用 | `sysctl net.inet.ip.fw.enable=0` |
| 模式查询 | `sysctl -n net.inet.ip.fw.default_to_accept` |
| 加载模块 | `kldload ipfw` |

### 9.2 pfctl 命令

| 操作 | 命令 |
|---|---|
| 列出规则 | `pfctl -sr` |
| 列出 NAT | `pfctl -sn` |
| 列出表 | `pfctl -s Tables` |
| 状态信息 | `pfctl -s info` |
| 加载规则 | `pfctl -f /etc/pf.conf` |
| 语法检查 | `pfctl -n -f /etc/pf.conf` |
| 启用 | `pfctl -e` |
| 禁用 | `pfctl -d` |
| 加载模块 | `kldload pf` |

### 9.3 使用的二进制路径（const 常量）

```rust
const IPFW: &str = "/sbin/ipfw";
const PFCTL: &str = "/sbin/pfctl";
const KLDLOAD: &str = "/sbin/kldload";
const KLDSTAT: &str = "/sbin/kldstat";
const SYSCTL: &str = "/sbin/sysctl";
const SYSRC: &str = "/usr/sbin/sysrc";   // 复用现有 sysrc 模块
const SH: &str = "/bin/sh";
```

---

## 10. P1 已知限制

以下能力 P1 尚未覆盖，不影响核心过滤功能：

| 项 | 说明 |
|---|---|
| **规则可达性分析** | 不检测规则冲突或冗余。 |
| **IPv6 完整支持** | 地址校验支持 IPv6，但 ICMPv6 等特殊协议处理可能不完整。 |
| **规则集（ipfw sets）** | 不使用 ipfw 的 set 机制（sets 0-31）做多组规则管理。 |
| **并发控制** | 同一时间只允许一个 firewall 写操作（通过 DB 事务 + 前端 loading 状态保证）。 |

---

## 11. P2 增强计划

> P1（M1-M6）已完成。以下为 P2 增强功能的详细设计计划，按优先级排列。

### 11.1 防锁死机制（P2-A，优先级：最高）

**问题**：当前 apply/switch/enable 直接修改内核规则，如果规则集阻断了管理连接（SSH 或面板 HTTP），用户将无法远程恢复，只能通过物理控制台操作。

**设计**：

1. **安全应用流程（safe apply）**：
   - 应用前备份当前配置文件和运行状态（见 §11.4）。
   - 写入临时配置并通过语法预检（`pfctl -n -f` / `ipfw -n`），失败则不应用。
   - 应用后启动「确认计时器」（默认 60 秒）。
   - 计时器未确认 → 自动恢复备份配置和运行状态。
   - 计时器确认 → 清除备份，应用生效。

2. **保底规则注入**：
   - apply/switch/enable 时自动注入临时放行规则：
     - 当前客户端 IP（从 HTTP 请求提取）放行面板端口。
     - SSH 端口（自动检测或配置指定）放行。
   - 保底规则标记为 `fwp-safety`，确认后移除（用户规则中已有等价放行时）或保留（由用户确认）。

3. **倒计时确认 UI**：
   - 前端弹出全屏倒计时对话框，显示剩余时间和「确认规则有效」/「立即回滚」按钮。
   - 倒计时归零等同「立即回滚」。

4. **一键禁用保留**：
   - 设置页面始终提供「紧急禁用」按钮（已有），确保即使其他页面不可达也能快速关闭防火墙。

**涉及文件**：
- 后端：`firewall_gen.rs`（safe apply 流程）、`handlers/firewall.rs`（confirm/rollback API）。
- 前端：`FirewallSettingsPage.vue` 或新 `FirewallApplyConfirm.vue`（倒计时对话框）。
- DB：新增 `firewall_apply_locks` 表（记录待确认的应用操作及其备份引用）。

### 11.2 配置验证与引用完整性（P2-B，优先级：高）

**问题**：当前规则保存时不检查协议与端口组合、IP 表引用是否存在，用户可能保存出无效或引用悬空的规则，直到 apply 时才失败。

**设计**：

1. **协议-端口一致性校验**（保存时）：
   - 仅 TCP/UDP 允许设置端口；ICMP/ICMPv6 禁止端口，要求 ICMP 类型。
   - 保存时校验，不通过则返回明确错误。

2. **IP 表引用完整性**：
   - 规则的 `source`/`destination` 若 `kind=table`，校验引用的表 ID 存在。
   - 删除 IP 表前检查引用：被引用时拒绝删除并返回引用规则列表，或要求用户确认后级联清除引用。
   - `firewall_tables` 删除时可选 `cascade` 模式：将引用规则的 `kind` 回退为 `any`。

3. **地址族一致性**：
   - 源和目的的 AF 不一致时给出警告（如 IPv4 源到 IPv6 目的永远不匹配）。
   - ICMP 类型与 AF 绑定（ICMP→inet，ICMPv6→inet6），保存时强制。

4. **端口与地址规格校验增强**：
   - 使用标准 IP/CIDR 解析器替代正则，拒绝如 `999.999.999.999` 的无效 IP。
   - 端口范围校验：下限 ≤ 上限，单端口 1-65535。

5. **前端即时校验**：
   - 规则编辑对话框中，协议切换时动态禁用/启用端口输入框。
   - 保存前前端预校验，减少无效 API 请求。

**涉及文件**：
- 后端：`firewall_gen.rs`（校验函数）、`db.rs`（引用检查查询）。
- 前端：`FirewallRulesPage.vue`（表单条件显示）、`FirewallTablesPage.vue`（删除确认）。

### 11.3 可观测性（P2-C，优先级：高）

**问题**：当前界面只展示 DB 规则和布尔状态，用户无法看到内核实际加载的规则、命中计数、状态表和加载错误。

**设计**：

1. **规则命中统计**：
   - ipfw：`ipfw -a -d -t list` 显示每条规则的包数/字节数/最后命中时间。
   - pf：`pfctl -v -sr` 显示规则评估计数。
   - 规则列表增加「命中包数」「字节数」「最后命中」列，可选展开。

2. **运行状态诊断面板**：
   - 展示内核实际规则（`pfctl -sr` / `ipfw list`），与 DB 期望规则对比，标记差异。
   - 显示模块加载状态、运行启用状态、最近 apply 的结果和错误信息。
   - PF：展示状态表摘要（`pfctl -ss` 摘要：活动状态条目数、TOP N 连接）。
   - PF：展示接口信息（`pfctl -s info`：运行时长、当前状态条目数、内存占用）。

3. **一致性检查**：
   - 对比三层状态：「DB 配置」「磁盘配置文件」「内核实际规则」。
   - 不一致时高亮警告（如手动改了配置文件后未 apply，或 apply 失败但文件已写入）。

4. **日志查看**：
   - 展示最近 N 条防火墙相关审计日志（apply/switch/enable/disable 及结果）。
   - 可选：对接系统防火墙日志（`/var/log/security` 或 ipfw LOG 目标）。

**涉及文件**：
- 后端：`firewall_gen.rs`（诊断采集函数）、`handlers/firewall.rs`（诊断 API）。
- 前端：新增诊断面板组件或 `FirewallRulesPage.vue` 中增加诊断标签页。

### 11.4 备份恢复与变更历史（P2-D，优先级：中）

**问题**：当前 apply 直接覆盖 `/etc/pf.conf` 和 `/etc/ipfw.rules`，无版本快照，误操作后无法回退。

**设计**：

1. **自动备份**：
   - 每次 apply、switch、enable、disable 前，自动备份：
     - `/etc/pf.conf` → `/var/db/fwp/backups/pf.conf.{timestamp}`
     - `/etc/ipfw.rules` → `/var/db/fwp/backups/ipfw.rules.{timestamp}`
     - 涉及的 rc.conf 键值快照
     - DB 规则快照（JSON 序列化全部规则 + 状态）
   - 备份记录存入 `firewall_backups` 表（时间戳、操作类型、操作者、文件路径、DB 快照）。

2. **版本浏览与差异**：
   - 界面展示备份历史列表（时间、操作、操作者）。
   - 可查看每个版本的配置文件内容和 DB 规则快照。
   - 可对比任意两个版本的差异（规则增删改、配置文件 diff）。

3. **一键恢复**：
   - 选择某个备份版本，恢复配置文件 + DB 规则 + rc.conf。
   - 恢复后可选自动 apply 使恢复立即生效。
   - 恢复操作本身也记录审计日志。

4. **导出备份**：
   - 可下载备份包（配置文件 + DB 快照的 ZIP/TAR），便于离线保存或迁移。

**涉及文件**：
- 后端：`db.rs`（`firewall_backups` 表）、`firewall_gen.rs`（备份/恢复函数）、`handlers/firewall.rs`（备份 API）。
- 前端：新增 `FirewallBackupsPage.vue` 或在设置页面增加备份管理标签。

### 11.5 NAT 与端口转发（P2-E，优先级：中）

> 已展开为独立设计文档：[19-nat.md](19-nat.md)。以下为原始概要，详细方案见新文档。

**问题**：当前仅支持过滤规则，不支持 NAT/重定向。Jail 和 Bhyve 场景常需要端口转发和源地址转换。

**设计**：

1. **独立 NAT 规则模型**（不复用过滤规则结构）：
   - NAT 规则字段：类型（SNAT/源地址转换、DNAT/端口转发、BINAT）、方向、外部接口、内部地址/网段、外部地址、协议、端口映射（原端口→目标地址:目标端口）。
   - 独立的 `firewall_nat_rules` 表。

2. **PF 生成**：
   - SNAT：`nat on $ext_if from $internal to any -> ($ext_if)`
   - DNAT：`rdr on $ext_if proto tcp from any to any port 80 -> 10.0.0.2 port 8080`
   - 生成到独立的 NAT 段（`/etc/pf.conf` 的 `nat-rdr` 区域），与过滤规则分离。

3. **IPFW 生成**：
   - 使用 `nat` 命令 + `divert` 或 `ipfw nat` 配置：
     ```sh
     ipfw -q nat 1 config if em0 same_ports reset
     ipfw -q add 1000 nat 1 ip from 10.0.0.0/24 to any out xmit em0
     ```
   - 需加载 `ipfw_nat` 模块（`kldload ipfw_nat`）。

4. **前端 NAT 管理页面**：
   - 在防火墙菜单下新增「NAT / 转发」子页面。
   - 独立的规则列表和编辑表单，与过滤规则页面分离。
   - 配置预览展示生成的 NAT 规则段。

5. **NAT 仅在支持该功能的驱动上启用**：
   - 两种驱动都支持 NAT，但语法和能力有差异，界面需说明当前驱动的 NAT 能力。

**涉及文件**：
- 后端：`db.rs`（`firewall_nat_rules` 表）、`firewall_gen.rs`（NAT 模型 + 生成器）、`handlers/firewall.rs`（NAT API）。
- 前端：新增 `FirewallNatPage.vue`。
- 菜单：`menu.js` 增加 NAT 子项。

### 11.6 规则导入导出（P2-F，优先级：中）

**问题**：当前无法备份规则集到外部文件，也无法从已有配置文件或另一台机器导入规则。

**设计**：

1. **导出**：
   - 导出为 JSON（结构化格式，包含全部过滤规则、NAT 规则、IP 表、状态配置）。
   - 可选：同时导出生成的原生配置文件（`pf.conf` / `ipfw.rules`）供参考。
   - 导出文件可作为另一台机器的导入源。

2. **导入**：
   - 导入 JSON：解析后写入 DB，替换或合并现有规则集。
   - 导入原生配置文件（`pf.conf` / `ipfw.rules`）：解析已有配置文件中的规则，转换为结构化规则存入 DB。
     - PF 解析：识别 `pass`/`block`/`quick`/`proto`/`from`/`to`/`port` 等关键字，映射到 `FirewallRule`。
     - IPFW 解析：识别 `add N allow/deny proto from ... to ...` 语法。
     - 无法精确映射的规则标记为「手动规则」，保留原文注释，不纳入结构化编辑。
   - 导入前预览：展示解析结果，用户确认后写入。

3. **冲突处理**：
   - 导入时可选模式：覆盖（清空现有后导入）、合并（追加，手动排序去重）。
   - 导入操作记录审计日志。

4. **前端导入导出页面**：
   - 设置页面增加「导入」「导出」按钮。
   - 导入支持文件上传 + 预览确认。
   - 导出支持下载 JSON 文件。

**涉及文件**：
- 后端：`firewall_gen.rs`（导入解析器、导出序列化）、`handlers/firewall.rs`（导入导出 API + 文件上传）。
- 前端：`FirewallSettingsPage.vue` 增加导入导出操作区。

---

## 12. P2 实现里程碑

| 里程碑 | 内容 | 预计工作量 | 依赖 |
|---|---|---|---|
| **M7 — 防锁死机制（P2-A）** | safe apply 流程、保底规则注入、倒计时确认 UI、自动回滚 | 大 | 无 |
| **M8 — 配置验证与引用完整性（P2-B）** | 协议-端口校验、IP 表引用检查、地址族一致性、前端即时校验 | 中 | 无 |
| **M9 — 可观测性（P2-C）** | 规则命中统计、运行状态诊断面板、一致性检查、日志查看 | 中 | 无 |
| **M10 — 备份恢复与变更历史（P2-D）** | 自动备份、版本浏览与差异、一键恢复、导出备份 | 中 | 建议在 M7 之后（M7 的回滚依赖备份） |
| **M11 — NAT 与端口转发（P2-E）** | NAT 规则模型、PF/IPFW NAT 生成器、NAT 管理页面 | 大 | 无 |
| **M12 — 规则导入导出（P2-F）** | JSON 导入导出、原生配置文件解析、冲突处理、前端 UI | 中 | 无 |
