# 模块设计：NAT 与端口转发

> 防火墙模块（[18-firewall.md](18-firewall.md)）的 P2-E / M11 增强功能。
> 依托已实现的防火墙框架（双驱动、staging、防锁死、配置生成），新增 NAT 规则的独立 CRUD 与生成器。
> 与原 P2-E 计划的区别：明确「独立模型 + 嵌入式生成」混合策略，复用现有 staging/apply/防锁死链路，避免重复造轮子。

---

## 1. 目标与意义

### 1.1 为什么面板需要 NAT

FreeBSD Web Panel 的核心场景之一是**容器/虚拟机管理**（Jail + Bhyve）。这些工作负载几乎都依赖 NAT：

| 场景 | NAT 类型 | 说明 |
|---|---|---|
| **Jail 出站访问** | SNAT | 非 vnet jail 共享主机网络栈时，源地址需转换为外网接口地址才能访问互联网 |
| **Bhyve VM 出站** | SNAT | vm-bhyve 默认通过 `ngctl` 交换机组网，VM 流量经主机 SNAT 出公网 |
| **Jail 服务暴露** | DNAT（端口转发） | 把主机 80/443 转发到 jail `10.0.0.2:80`，外部可访问容器内服务 |
| **VM 服务暴露** | DNAT | 同上，将主机端口映射到 VM 内部 IP:port |
| **IPv4 共享** | SNAT | 公网 IPv4 资源稀缺，多个容器/VM 共享主机单一公网 IP 是常态 |

**没有 NAT 的后果**：用户必须手动编辑 `/etc/pf.conf` 或 `/etc/ipfw.rules`，面对易错的语法（pf 的 `nat on` / `rdr on`、ipfw 的 `nat N config if` + `divert` 或 `ipfw nat`），且与面板管理的过滤规则容易冲突——面板下次 apply 会覆盖手写的 NAT 段。

### 1.2 与现有功能的关系

```
┌──────────────────────────────────────────────────────────────┐
│                  NAT 在系统中的位置                            │
│                                                              │
│   ┌──────────┐    ┌──────────────┐    ┌──────────────────┐   │
│   │  Jail    │───▶│   内部网段    │───▶│   主机 NAT 引擎  │   │
│   │  Bhyve   │    │ 10.0.0.0/24  │    │  (pf / ipfw)     │   │
│   └──────────┘    └──────────────┘    └────────┬─────────┘   │
│                                                │             │
│                                       ┌────────▼─────────┐   │
│                                       │   外网接口 em0    │   │
│                                       │   (默认路由出口)  │   │
│                                       └──────────────────┘   │
│                                                              │
│   控制平面：                                                 │
│   ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│   │ 防火墙模块    │  │ NAT 子模块    │  │ Jail/Bhyve 模块  │  │
│   │ (filter 规则) │  │ (nat 规则)   │  │ (联动快捷操作)   │  │
│   └──────────────┘  └──────────────┘  └──────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

- **与防火墙**：共用配置文件（`/etc/pf.conf` / `/etc/ipfw.rules`），共用驱动状态、apply/staging/防锁死链路
- **与 Jail/Bhyve**：典型内部网段（`10.0.0.0/24`、vm-bhyve 交换机网段）由这些模块产生；P2 提供「为该 jail 配置端口转发」的快捷入口
- **与网络接口**：外部接口通过 `/api/network/gateway` 已返回的 `interface` 字段识别（默认路由出口）

### 1.3 设计目标

| 目标 | 说明 |
|---|---|
| **独立模型** | NAT 字段集与过滤规则差异大（外部接口、内部网段、目标地址:端口），不复用 `FirewallRule` |
| **嵌入式生成** | NAT 段嵌入现有配置文件，不创建独立文件，避免 `pfctl -f` 多次加载 |
| **零重复造轮** | 复用 staging（暂存未应用变更）、防锁死（备份+回滚）、apply（生成+加载）、倒计时确认 |
| **驱动对等** | pf 和 ipfw 都支持，但能力差异在 UI 中明确告知 |
| **与现有 CRUD 解耦** | NAT 规则独立列表、独立 API、独立前端页面，不影响过滤规则管理 |

### 1.4 P1 / P2 划分

| 期 | 范围 |
|---|---|
| **P1 核心** | SNAT + DNAT 数据模型、CRUD、PF/IPFW 生成、独立页面、与 staging/apply/防锁死集成 |
| **P2 增强** | BINAT（1:1 映射）、Jail/Bhyve 联动快捷操作、NAT 状态查看、端口冲突检测、NAT 命中统计 |

---

## 2. 双驱动 NAT 能力对比

| 能力 | pf | ipfw |
|---|---|---|
| **SNAT（出站源转换）** | `nat on $ext from $src to any -> ($ext)` | `nat 1 config if $ext same_ports reset` + `add N nat 1 ip from $src to any out` |
| **DNAT（端口转发）** | `rdr on $ext proto tcp to any port 80 -> 10.0.0.2 port 8080` | `nat 1 config if $ext redirect 10.0.0.2:8080` + `add N nat 1 tcp from any to me 80 in` |
| **BINAT（1:1 映射）** | `binat on $ext from $ip to any -> $ext_ip` | 不直接支持（需多条规则模拟） |
| **模块依赖** | 内置（`pf.ko`） | 需 `ipfw_nat.ko`（`kldload ipfw_nat`） |
| **状态保持** | 自动 `keep state` | 需配合 `keep-state` |
| **端口范围** | `port 80:443` | `80-443` |
| **多对一（MASQUERADE）** | 默认行为（`-> ($ext_if)`） | `same_ports` 选项 |
| **重定向到本机** | `rdr ... -> 127.0.0.1 port ...` | `redirect 127.0.0.1:port` |

**面板策略**：
- 两种驱动都暴露 SNAT + DNAT
- BINAT 仅在 pf 驱动下显示（ipfw 隐藏该选项 + 提示说明）
- ipfw NAT 需要 `kldload ipfw_nat`，初始化 NAT 时检查并加载（与 `ensure_module` 同模式）

---

## 3. 整体架构

### 3.1 数据流

```
┌──────────────┐
│  前端表单    │  POST /api/firewall/nat/rules
│ (NAT 子页面) │ ─────────────────────────────────┐
└──────────────┘                                   ▼
                                          ┌──────────────────┐
                                          │ handlers/firewall│
                                          │ ::nat_* handlers │
                                          └────────┬─────────┘
                                                   │
                                  ┌────────────────┴────────────────┐
                                  │                                 │
                            FW 已禁用                            FW 已启用
                                  │                                 │
                                  ▼                                 ▼
                          DB INSERT/UPDATE              写 staging（nat_rules 字段）
                          + regen_config()              + 列表读 staging
                                  │                                 │
                                  └────────────┬────────────────────┘
                                               │
                                               ▼
                                  apply / confirm / rollback
                                  (复用现有防锁死链路)
                                               │
                                               ▼
                          generate_pf / generate_ipfw
                          (在过滤规则前嵌入 NAT 段)
                                               │
                                               ▼
                          pfctl -f / sh /etc/ipfw.rules
```

### 3.2 模块结构（新增/改动文件）

```
src/
├── firewall_gen.rs          # ← 扩展：NatRule 类型、CRUD、generate_pf_nat / generate_ipfw_nat
├── handlers/firewall.rs     # ← 扩展：nat_* handlers + 路由
├── app.rs                   # ← 扩展：注册 /api/firewall/nat/* 路由
└── db.rs                    # ← 扩展：migrate() 新增 firewall_nat_rules 表

frontend/src/
├── pages/
│   └── FirewallNatPage.vue  # ★ 新增：NAT 规则管理页面
├── lib/menu.js              # ← 扩展：firewall 子菜单加 'NAT / Port Forward'
├── i18n/translations.js     # ← 扩展：nav.firewallNat + NAT 字段翻译
└── components/ui/DialogHost.vue  # 复用现有 form 模式（radio-pill/select/row 布局）
```

### 3.3 与防火墙各子系统的集成点

| 子系统 | NAT 接入方式 |
|---|---|
| **驱动选择/切换** | 无需改动——NAT 规则与过滤规则共用同一 `active_driver` |
| **模式切换（黑白名单）** | 无需改动——NAT 段独立于默认策略 |
| **启用/禁用** | 无需改动——NAT 规则嵌入配置文件，随防火墙启用而生效 |
| **Apply 流程** | `apply_pf` / `apply_ipfw` 调用生成器时传入 NAT 规则；备份/回滚自动覆盖（配置文件含 NAT 段） |
| **Staging** | `StagingData` 加 `nat_rules` 字段；`write_staging` / `read_staging` / `replace_all_*` 全量同步 |
| **防锁死（pending）** | 无需改动——`backup_config` 是配置文件全文快照，已包含 NAT 段；回滚时整体恢复 |
| **配置预览** | `GET /api/firewall/config` 已返回完整配置文件，NAT 段自动包含 |

> **关键决策**：NAT 不引入新的「应用」按钮——所有 NAT 变更与过滤规则变更共用同一个 apply/confirm/rollback 流程。这避免了「过滤规则已应用但 NAT 未应用」的不一致状态。

---

## 4. 数据模型

### 4.1 Rust 结构体（`firewall_gen.rs`）

```rust
/// NAT 规则类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatKind {
    Snat,   // 出站源地址转换（MASQUERADE）
    Dnat,   // 入站端口转发（REDIRECT）
    Binat,  // 1:1 双向映射（仅 pf；P2）
}

/// NAT 规则的地址族
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatFamily {
    Ip,     // IPv4（pf: inet；ipfw: 默认）
    Ip6,    // IPv6（pf: inet6）
}

/// 结构化 NAT 规则——驱动无关抽象。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NatRule {
    pub id: i64,
    pub position: u32,
    pub enabled: bool,

    pub kind: NatKind,
    pub family: NatFamily,

    // 接口
    pub interface: String,          // 外部接口（如 "em0"），必填

    // 源/目标地址（语义随 kind 变化）
    // SNAT: src = 内部网段（如 "10.0.0.0/24"）；dst 通常为 any
    // DNAT: src 通常为 any；原始端口 = src_port_orig，目标 = dst_addr:dst_port
    // BINAT: src = 内部 IP；dst = 外部 IP（一对一）
    pub src_addr: String,           // CIDR 或 IP，必填
    pub dst_addr: Option<String>,   // SNAT 时可为空（= 接口地址）；DNAT 时为目标 IP

    // 端口（仅 DNAT / 限定端口的 SNAT）
    pub src_port: Option<String>,   // 原端口（DNAT 必填；SNAT 可选限定源端口）
    pub dst_port: Option<String>,   // 目标端口（DNAT 转发后的端口）

    // 协议（TCP/UDP；Any 表示两者都）
    pub protocol: NatProto,

    // 元数据
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatProto {
    Tcp,
    Udp,
    Both,   // TCP + UDP
}

/// 创建/更新请求体
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NatBody {
    pub kind: NatKind,
    pub family: NatFamily,
    pub interface: String,
    pub src_addr: String,
    #[serde(default)]
    pub dst_addr: Option<String>,
    #[serde(default)]
    pub src_port: Option<String>,
    #[serde(default)]
    pub dst_port: Option<String>,
    pub protocol: NatProto,
    #[serde(default)]
    pub enabled: bool,              // 默认 true
    #[serde(default)]
    pub description: Option<String>,
}
```

### 4.2 SQLite 表

```sql
CREATE TABLE firewall_nat_rules (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    position    INTEGER NOT NULL DEFAULT 0,
    enabled     INTEGER NOT NULL DEFAULT 1,
    kind        TEXT    NOT NULL,           -- 'snat' | 'dnat' | 'binat'
    family      TEXT    NOT NULL,           -- 'ip' | 'ip6'
    interface   TEXT    NOT NULL,           -- 外部接口名
    src_addr    TEXT    NOT NULL,
    dst_addr    TEXT,
    src_port    TEXT,
    dst_port    TEXT,
    protocol    TEXT    NOT NULL,           -- 'tcp' | 'udp' | 'both'
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX idx_firewall_nat_position ON firewall_nat_rules(position);
```

> **与 `firewall_rules` 解耦**：NAT 规则与过滤规则的 position 独立排序，互不影响。

---

## 5. 后端实现

### 5.1 DB CRUD（`firewall_gen.rs`）

仿照 `firewall_rules` 的 CRUD 模式：

| 函数 | 签名 |
|---|---|
| `list_nat_rules(&conn) -> Vec<NatRule>` | 按 position 升序 |
| `next_nat_position(&conn) -> u32` | 复用 `next_position` 的溢出修复模式（i64 计算后转 u32） |
| `create_nat_rule(&conn, &NatBody, now) -> i64` | INSERT |
| `update_nat_rule(&conn, id, &NatBody, now) -> ()` | UPDATE（NotFound if 0） |
| `delete_nat_rule(&conn, id) -> ()` | DELETE |
| `toggle_nat_rule(&conn, id) -> ()` | 启用/禁用 |
| `reorder_nat_rules(&conn, &[i64]) -> ()` | 批量更新 position |
| `replace_all_nat_rules(&conn, &[NatRule]) -> ()` | staging confirm 时全量替换 |

### 5.2 配置生成

**PF — NAT 段嵌入到 `generate_pf`**：

```pf
# ============================================================
# Managed by FreeBSD Web Panel (fwp) - DO NOT EDIT MANUALLY
# Driver: pf | Mode: whitelist (default deny)
# ============================================================

# --- IP Tables ---
table <...>

# --- NAT / RDR ---                   ← 新增段，在 block all 之前
nat on em0 inet from 10.0.0.0/24 to any -> (em0)
rdr on em0 inet proto tcp from any to any port 80 -> 10.0.0.2 port 8080

# Default policy: block all inbound, allow all outbound (whitelist mode)
set skip on lo0
block all
pass out quick all flags any keep state (sloppy)

# --- Filter Rules ---
pass quick in inet proto tcp ...
```

**关键点（PF）**：
1. NAT/rdr 段必须在 `block all` **之前**——PF 按规则顺序评估，NAT 在过滤前生效
2. SNAT 用 `nat on $if $af from $src to any -> ($if)` —— `($if)` 表示接口当前地址（DHCP 场景自动跟随）
3. DNAT 用 `rdr on $if $af proto X from any to any port $port -> $target port $tport`
4. 端口范围：`port 80:443`（用 `port_to_pf` 现有转换）
5. BINAT（P2）：`binat on $if from $ip to any -> $ext_ip`

**ipfw — NAT 段嵌入到 `generate_ipfw`**：

```sh
#!/bin/sh
# Managed by FreeBSD Web Panel (fwp) - DO NOT EDIT MANUALLY
# Driver: ipfw | Mode: whitelist (default deny)
# ============================================================

ipfw -q flush

# --- IP Tables ---
ipfw -q table NAME flush
...

# --- NAT configuration ---           ← 新增段
# [SNAT] NAT for 10.0.0.0/24 via em0
ipfw -q nat 1 config if em0 same_ports reset
ipfw -q nat 2 config if em0 redirect 10.0.0.2:8080 tcp

# --- Managed Rules ---
ipfw -q add 00100 allow tcp from any to any dst-port 80 in

# --- NAT rules (must come after filter rules) ---   ← 新增段
# [SNAT] Apply nat 1 to outbound from 10.0.0.0/24
ipfw -q add 50000 nat 1 ip from 10.0.0.0/24 to any out xmit em0
# [DNAT] Redirect inbound TCP/80 to 10.0.0.2:8080
ipfw -q add 50100 nat 2 tcp from any to me 80 in

# [65000] Allow all outbound (whitelist mode)
ipfw -q add 65000 allow ip from any to any out keep-state
# [65534] Default deny (whitelist mode)
ipfw -q add 65534 deny ip from any to any
```

**关键点（ipfw）**：
1. `nat N config` 必须在使用 `nat N` 的规则**之前**声明（实例配置）
2. NAT 规则编号从 `50000` 开始，步进 `100`（与过滤规则的 `00100-49900` 隔离）
3. SNAT 用 `same_ports reset` 选项（避免端口冲突，行为类似 MASQUERADE）
4. DNAT 需要 `redirect` 配置 + `nat N tcp from any to me PORT in` 规则
5. **模块加载**：`init_ipfw` / `apply_ipfw` 时检查并 `kldload ipfw_nat`（如未加载）

**生成器伪代码**：

```rust
pub fn generate_pf_nat(rules: &[NatRule]) -> String {
    let mut buf = String::new();
    let active: Vec<&NatRule> = rules.iter().filter(|r| r.enabled).collect();
    if active.is_empty() {
        return buf;
    }
    buf.push_str("# --- NAT / RDR ---\n");
    for rule in active {
        let af = match rule.family { NatFamily::Ip => "inet", NatFamily::Ip6 => "inet6" };
        let proto = match rule.protocol {
            NatProto::Tcp => " proto tcp",
            NatProto::Udp => " proto udp",
            NatProto::Both => " proto { tcp udp }",
        };
        let desc = rule.description.as_deref().unwrap_or("");
        match rule.kind {
            NatKind::Snat => {
                let target = rule.dst_addr.as_deref()
                    .map(|s| s.as_str())
                    .unwrap_or(&format!("({})", rule.interface));
                buf.push_str(&format!(
                    "# [SNAT] {desc}\nnat on {iface} {af} from {src} to any -> {target}\n",
                    iface = rule.interface, af = af, src = rule.src_addr, target = target,
                ));
            }
            NatKind::Dnat => {
                let dport = rule.src_port.as_deref()
                    .map(|p| format!(" port {}", port_to_pf(p))).unwrap_or_default();
                let target = rule.dst_addr.as_deref().unwrap_or("127.0.0.1");
                let tport = rule.dst_port.as_deref()
                    .map(|p| format!(" port {}", p)).unwrap_or_default();
                buf.push_str(&format!(
                    "# [DNAT] {desc}\nrdr on {iface} {af}{proto} from any to any{dport} -> {target}{tport}\n",
                    iface = rule.interface, af = af, proto = proto,
                    dport = dport, target = target, tport = tport,
                ));
            }
            NatKind::Binat => { /* P2 */ }
        }
    }
    buf.push('\n');
    buf
}

pub fn generate_ipfw_nat(rules: &[NatRule]) -> (String, String) {
    // 返回 (config段, rules段)
    // config 段：nat N config if ...
    // rules 段：ipfw -q add 50000 nat N ...
    // ...
}
```

**`generate_pf` / `generate_ipfw` 修改点**：

```rust
pub fn generate_pf(
    rules: &[FirewallRule],
    mode: FirewallMode,
    tables: &[IpTable],
    nat_rules: &[NatRule],      // ← 新增参数
) -> String {
    let mut buf = header(FirewallDriver::Pf, mode);
    // ... tables 段 ...
    buf.push_str(&generate_pf_nat(nat_rules));  // ← 嵌入 NAT 段（在 block all 之前）
    // ... 默认策略 + 过滤规则 ...
    buf
}

pub fn generate_ipfw(
    rules: &[FirewallRule],
    mode: FirewallMode,
    tables: &[IpTable],
    nat_rules: &[NatRule],      // ← 新增参数
) -> String {
    let mut buf = header(FirewallDriver::Ipfw, mode);
    buf.push_str("ipfw -q flush\n\n");
    // ... tables 段 ...
    let (nat_config, nat_rules_str) = generate_ipfw_nat(nat_rules);
    buf.push_str(&nat_config);                  // ← nat N config 声明
    // ... 过滤规则 ...
    buf.push_str(&nat_rules_str);               // ← nat N 规则（在过滤规则后）
    // ... 默认策略 65534 ...
    buf
}
```

### 5.3 ipfw_nat 模块加载

```rust
/// Check if ipfw_nat module is loaded.
pub fn ipfw_nat_loaded() -> bool {
    cmd::status_sync(KLDSTAT, &["-q", "-n", "ipfw_nat"])
}

/// Ensure ipfw_nat is loaded (called by apply_ipfw when NAT rules exist).
pub fn ensure_ipfw_nat() -> ApiResult<()> {
    if ipfw_nat_loaded() {
        return Ok(());
    }
    cmd::run_sync(KLDLOAD, &["ipfw_nat"])?;
    Ok(())
}
```

`apply_ipfw` 修改：若 `nat_rules` 非空，先 `ensure_ipfw_nat()`，再生成+加载。

### 5.4 输入校验

| 字段 | 规则 |
|---|---|
| `interface` | 匹配 `^[a-zA-Z0-9_.]{1,15}$`（与过滤规则一致） |
| `src_addr` / `dst_addr` | 合法 IPv4/IPv6 或 CIDR；family 一致（family=ip 时不允许 IPv6 地址） |
| `src_port` / `dst_port` | 匹配 `^(\d+)(-(\d+))?(,(\d+)(-(\d+))?)*$`，1-65535 |
| `kind=dnat` | `src_port` 必填（原端口）；`dst_addr` 必填（目标 IP） |
| `kind=snat` | `dst_addr` 可空（= 接口地址） |
| `kind=binat` | 仅 `family=ip` + `driver=pf` 时允许（P2） |
| `description` | ≤ 200 字符，无换行 |

### 5.5 外部接口自动检测

复用 `/api/network/gateway` 已返回的 `interface` 字段（默认路由出口）。前端 NAT 表单的「外部接口」字段默认填入此值，用户可手动改为其他接口（如 bridge0、vpn0）。

---

## 6. API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/firewall/nat/rules` | 列出所有 NAT 规则（按 position 升序） |
| POST | `/api/firewall/nat/rules` | 添加 NAT 规则 |
| PUT | `/api/firewall/nat/rules/{id}` | 修改 NAT 规则 |
| DELETE | `/api/firewall/nat/rules/{id}` | 删除 NAT 规则 |
| PUT | `/api/firewall/nat/rules/{id}/toggle` | 启用/禁用 |
| PUT | `/api/firewall/nat/rules/reorder` | 重排序 `{ ordered_ids: [...] }` |

> 配置预览、apply、confirm、rollback 复用 `/api/firewall/config`、`/api/firewall/apply` 等（已存在，自动包含 NAT 段）。

**请求示例**：

```json
// POST /api/firewall/nat/rules — 添加 SNAT（为 jail 网段提供出口）
{
    "kind": "snat",
    "family": "ip",
    "interface": "em0",
    "src_addr": "10.0.0.0/24",
    "protocol": "both",
    "description": "NAT for jail network"
}

// 响应 201
{
    "id": 1,
    "position": 0,
    "enabled": true,
    "kind": "snat",
    "family": "ip",
    "interface": "em0",
    "src_addr": "10.0.0.0/24",
    "dst_addr": null,
    "src_port": null,
    "dst_port": null,
    "protocol": "both",
    "description": "NAT for jail network",
    "created_at": 1705312200,
    "updated_at": 1705312200
}
```

```json
// POST /api/firewall/nat/rules — 添加 DNAT（把主机 80 转发到 jail 10.0.0.2:8080）
{
    "kind": "dnat",
    "family": "ip",
    "interface": "em0",
    "src_addr": "any",
    "dst_addr": "10.0.0.2",
    "src_port": "80",
    "dst_port": "8080",
    "protocol": "tcp",
    "description": "Forward HTTP to jail web-01"
}
```

---

## 7. 前端设计

### 7.1 菜单结构

`menu.js` 的 `/firewall` 子菜单新增一项：

```js
{
  path: '/firewall',
  labelKey: 'nav.pf',
  icon: 'fa-solid fa-shield-halved',
  children: [
    { path: '/firewall/rules',    labelKey: 'nav.firewallRules',    icon: 'fa-solid fa-list' },
    { path: '/firewall/nat',      labelKey: 'nav.firewallNat',      icon: 'fa-solid fa-arrow-right-arrow-left' },
    { path: '/firewall/tables',   labelKey: 'nav.firewallTables',   icon: 'fa-solid fa-table-list' },
    { path: '/firewall/settings', labelKey: 'nav.firewallSettings', icon: 'fa-solid fa-gear' },
  ],
},
```

### 7.2 页面布局（`FirewallNatPage.vue`）

复用 `FirewallRulesPage.vue` 的整体结构（状态卡片 + 列表 + 配置预览）：

```
┌─────────────────────────────────────────────────────────┐
│  NAT / 端口转发                          [刷新]          │
├─────────────────────────────────────────────────────────┤
│  ┌─ 状态卡片（共用 FirewallRulesPage 的逻辑）──────────┐ │
│  │  驱动: pf  状态: ● 运行中  NAT 规则数: 2           │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  ┌─ NAT 规则列表 ─────────────────────────────────────┐ │
│  │  [+ 添加 NAT 规则]              [应用变更]*        │ │
│  │  * 仅在防火墙已启用且有 staging 时显示              │ │
│  │                                                    │ │
│  │  #  类型  协议  接口  源           →  目标          │ │
│  │  ─  ────  ────  ────  ──────────     ──────────    │ │
│  │  1  SNAT  ANY   em0   10.0.0.0/24    (em0)         │ │
│  │  2  DNAT  TCP    em0   port 80       10.0.0.2:8080 │ │
│  │                                                    │ │
│  │  [启用/禁用] [编辑] [删除]                         │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  └─ 驱动能力提示（ipfw 时显示）─────────────────────────┘ │
│    ⚠ ipfw 的 NAT 需要 ipfw_nat 内核模块                 │
└─────────────────────────────────────────────────────────┘
```

### 7.3 规则编辑表单

使用 `useFormModal` + `DialogHost.vue` 的 form 类型，复用 FirewallRulesPage 的字段约定（radio-pill/select/checkbox/row 布局）：

```
┌─────────────────────────────────────────────────────────┐
│  添加 NAT 规则                                    [×]    │
├─────────────────────────────────────────────────────────┤
│  描述:   [Forward HTTP to jail web-01                 ] │
│                                                         │
│  类型:   [SNAT] [DNAT] (radio-pill)                     │
│                                                         │
│  地址族: [IPv4] [IPv6] (radio-pill)                     │
│                                                         │
│  协议:   [TCP ▼]    外部接口: [em0 ▼]   (row 同行)     │
│                                                         │
│  ─── SNAT 字段（kind=snat 时显示）─────────────────────│
│  源网段:   [10.0.0.0/24             ]                   │
│  目标地址: [(em0)                  ] (留空=接口地址)    │
│                                                         │
│  ─── DNAT 字段（kind=dnat 时显示）─────────────────────│
│  原端口:   [80                      ]                   │
│  目标地址: [10.0.0.2                ]                   │
│  目标端口: [8080                    ]                   │
│                                                         │
│  记录日志（暂不实现，预留）                              │
│                                                         │
│                              [取消]    [保存]            │
└─────────────────────────────────────────────────────────┘
```

**字段动态显示**：根据 `kind` 字段切换显示 SNAT / DNAT / BINAT 字段组，使用 `row` 属性排版。

**submitHandler 模式**：API 失败时错误内联显示在弹窗内，不丢失输入（与 FirewallRulesPage 一致）。

### 7.4 未初始化 / 防火墙未启用状态

- **防火墙未初始化**：显示「请先在设置页面初始化防火墙」提示 + 跳转按钮
- **防火墙已禁用**：CRUD 直接写 DB + regen_config（无 staging）
- **防火墙已启用**：CRUD 写 staging，列表显示 staging 内容，显示「应用变更」按钮

### 7.5 消息反馈策略

遵循项目约定（成功 → toast，失败 → 弹窗）：

| 场景 | 方式 |
|---|---|
| 规则添加/编辑/删除成功 | `useToast()` |
| 应用变更成功 | `useToast()` |
| 应用变更失败 | `useAlert()` / 表单内 errorMessage |
| BINAT 在 ipfw 下尝试保存 | `useAlert()`（提示驱动不支持） |

---

## 8. 安全考量

### 8.1 防锁死集成（关键）

NAT 规则变更可能阻断管理连接的常见场景：
- **错误的 DNAT**：把面板端口（18080）转发到内部地址，导致面板自身不可达
- **错误的 SNAT**：源地址转换错误，导致出站连接失败，影响管理通道

**缓解**：完全复用现有的防锁死机制（备份 → 应用 → 60s 倒计时 → 自动回滚）。NAT 段嵌入配置文件，备份/回滚自动覆盖。**无需新增逻辑**。

### 8.2 端口冲突检测（P2）

P1 不检测 DNAT 端口冲突（如两条规则都转发 80 端口）。P2 增加：
- 同一接口同一端口的 DNAT 规则重复时，保存时拒绝并提示
- 与过滤规则的「放行该端口入站」一致性检查（提示用户是否需要相应过滤规则）

### 8.3 ipfw_nat 模块加载失败处理

`apply_ipfw` 时若 NAT 规则非空但 `kldload ipfw_nat` 失败：
- 返回明确错误（不静默）
- 不应用规则（保持现有配置）
- 错误信息说明「NAT 规则需要 `ipfw_nat` 内核模块，加载失败——请确认内核配置」

### 8.4 接口名注入防护

与过滤规则一致——`interface` 字段严格匹配 `^[a-zA-Z0-9_.]{1,15}$`，所有系统命令通过 `Command::new().arg()` 传递，**禁止字符串拼接 shell**。

---

## 9. 命令参考

### 9.1 PF NAT 命令

| 操作 | 命令 |
|---|---|
| 列出 NAT 规则 | `pfctl -sn` |
| 列出 rdr 规则 | `pfctl -sn`（NAT + rdr 一起） |
| 查看 NAT 状态 | `pfctl -s state` |
| 重载 NAT 配置 | `pfctl -f /etc/pf.conf`（与过滤规则一同加载） |

### 9.2 ipfw NAT 命令

| 操作 | 命令 |
|---|---|
| 列出 NAT 配置 | `ipfw nat show config` |
| 列出 NAT 规则 | `ipfw list`（含 `nat N` 规则） |
| 查看 NAT 翻译 | `ipfw nat show` |
| 加载 NAT 模块 | `kldload ipfw_nat` |
| 检查模块 | `kldstat -q -n ipfw_nat` |

### 9.3 外部接口检测

复用现有 `/api/network/gateway` 返回的 `interface` 字段（默认路由出口）。

---

## 10. 实施计划

### 10.1 里程碑划分

| 里程碑 | 内容 | 预计工作量 | 依赖 |
|---|---|---|---|
| **N1 — 数据层** | `firewall_nat_rules` 表 + `NatRule` 类型 + DB CRUD（含 `replace_all_nat_rules`） | 小 | 无 |
| **N2 — 生成器** | `generate_pf_nat` / `generate_ipfw_nat` + 嵌入到 `generate_pf` / `generate_ipfw`（增加 `nat_rules` 参数） | 中 | N1 |
| **N3 — Handlers + 路由** | `nat_*` handlers（CRUD + toggle + reorder） + `app.rs` 路由注册 | 中 | N1 |
| **N4 — Staging 集成** | `StagingData` 加 `nat_rules` 字段；`write_staging` / `read_staging` / `replace_all_*` 同步；`effective_state` 返回 NAT 规则 | 中 | N1, N2 |
| **N5 — Apply 集成** | `apply_pf` / `apply_ipfw` 传入 NAT 规则；ipfw 时检查 `ipfw_nat` 模块 | 小 | N2 |
| **N6 — 前端页面** | `FirewallNatPage.vue` + 菜单项 + i18n + 路由 | 中 | N3 |
| **N7 — 测试与文档** | 端到端测试（jail 出站 + 端口转发场景）+ `docs/impl/28-nat.md` | 小 | N1-N6 |

### 10.2 实施顺序建议

1. **N1 + N3**（DB + Handlers）—— 先建立 CRUD 骨架，前端可以立即调试
2. **N2 + N5**（生成器 + Apply）—— 配置生成正确后再测试 apply
3. **N4**（Staging）—— 让 NAT 变更在防火墙启用时也安全
4. **N6**（前端）—— UI 整合
5. **N7**（测试 + 文档）—— 端到端验证 + 同步实现文档

### 10.3 与现有 staging/防锁死链路的兼容性

**关键不变量**：
- `StagingData` 加 `nat_rules` 字段后，旧 staging 文件（无该字段）反序列化时需默认空 `Vec`（`#[serde(default)]`）
- `replace_all_rules` / `replace_all_tables` / `replace_all_nat_rules` 三者在 confirm 时一起调用（同一事务）
- `backup_config` 不变（仍是配置文件全文快照）—— NAT 段自动包含

**破坏性变更（需注意）**：
- `generate_pf` / `generate_ipfw` 签名变化（加 `nat_rules` 参数）—— 所有调用方（apply、init、regen_config、preview_config、rollback）都需要更新

---

## 11. P2 增强计划

| 项 | 说明 |
|---|---|
| **BINAT（1:1 映射）** | 仅 pf 驱动支持；用于把一个外部 IP 完全映射到内部 IP（如 DMZ 主机） |
| **Jail/Bhyve 联动** | Jail 详情页提供「配置端口转发」按钮，预填 jail IP；Bhyve VM 同理 |
| **NAT 状态查看** | `pfctl -s state` / `ipfw nat show` 输出解析，展示当前活跃的 NAT 连接 |
| **NAT 命中统计** | `pfctl -v -sn` 显示每条 NAT 规则的包数/字节数 |
| **端口冲突检测** | 保存 DNAT 时检查端口已被占用（其他 NAT 规则或本机监听） |
| **预设模板** | 常见场景模板（Web 转发、SSH 转发、jail 网段出口）一键创建 |

---

## 12. 已知限制（P1）

| 项 | 说明 |
|---|---|
| **不支持 BINAT** | P2 实现（仅 pf 支持） |
| **无端口冲突检测** | 同一接口同一端口的 DNAT 重复时不报错（pf 用首条匹配） |
| **无 NAT 状态查看** | 不展示活跃 NAT 连接（P2） |
| **ipfw NAT 需要 ipfw_nat 模块** | 内核必须编译或加载 `ipfw_nat.ko`；否则 apply 失败 |
| **IPv6 SNAT 较少用** | IPv6 通常全局可达，NAT66 用例少；保留 family=ip6 选项供特殊场景 |
| **无规则导入** | 不解析已有 pf.conf 中的 `nat`/`rdr` 规则（P2-F 计划） |
| **外部接口变更未自动跟随** | 若默认路由接口变更（如 DHCP 切换），NAT 规则中的 interface 字段不自动更新 |
