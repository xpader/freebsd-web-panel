# 28 — NAT 与端口转发

## 概述

在防火墙模块（双驱动 ipfw / pf）的基础上新增 **NAT 与端口转发** 子功能。支持 SNAT（出站源地址转换）和 DNAT（入站端口转发），独立的 NAT 规则模型，配置生成时嵌入现有 `/etc/pf.conf` 或 `/etc/ipfw.rules`，与过滤规则共用同一套 staging/apply/防锁死链路。

**关键设计决策**：
- **独立数据模型** — NAT 字段集（外部接口、内部网段、目标地址:端口）与过滤规则差异大，不复用 `FirewallRule`，用独立的 `firewall_nat_rules` 表 + `NatRule` 结构体
- **嵌入式生成** — NAT 段嵌入现有配置文件，PF 放在 `block all` 之前，ipfw 用 `nat N config` 声明 + `nat N` 规则（编号 50000+），不创建独立文件
- **零重复造轮** — 复用 staging（暂存未应用变更）、防锁死（备份+回滚）、apply（生成+加载）、倒计时确认，不新增独立流程
- **与现有 CRUD 解耦** — NAT 规则独立列表、独立 API、独立前端页面（`/firewall/nat`）

## 实现细节

### 数据结构

**Rust 结构体**（`src/firewall_gen.rs`）：

```rust
enum NatKind { Snat, Dnat, Binat }         // 源转换 / 端口转发 / 1:1 映射
enum NatFamily { Ip, Ip6 }                  // IPv4 / IPv6
enum NatProto { Tcp, Udp, Both }            // 协议；Both = TCP+UDP

struct NatRule {
    id, position, enabled,
    kind: NatKind,
    family: NatFamily,
    interface: String,          // 外部接口（如 "em0"），必填
    src_addr: String,           // SNAT=内部网段；DNAT="any" 或限定源
    dst_addr: Option<String>,   // SNAT=空(=接口地址)；DNAT=内部目标 IP
    src_port: Option<String>,   // DNAT 必填（原端口）
    dst_port: Option<String>,   // DNAT 目标端口
    protocol: NatProto,
    description: Option<String>,
    created_at, updated_at,
}

struct NatBody { /* 创建/更新请求体，字段同 NatRule 但无 id/position/timestamps */ }
```

### SQLite 表

```sql
CREATE TABLE firewall_nat_rules (
    id, position, enabled,
    kind, family, interface,
    src_addr, dst_addr, src_port, dst_port,
    protocol, description, created_at, updated_at
);
-- 由 db.rs 的 m2 迁移创建
```

### 配置生成

**PF**（`generate_pf_nat`）— 在 `generate_pf` 的 IP tables 之后、`block all` 之前嵌入：

```pf
# --- NAT / RDR ---
# [SNAT] NAT for jail network
nat on em0 inet from 10.0.0.0/24 to any -> (em0)
# [DNAT] Forward HTTP to jail
rdr on em0 inet proto tcp from any to any port 80 -> 10.0.0.2 port 8080
```

**关键点**：
- NAT/rdr 段必须在 `block all` **之前**——PF 按规则顺序评估，NAT 在过滤前生效
- SNAT 用 `nat on $if $af from $src to any -> ($if)` —— `($if)` 表示接口当前地址（DHCP 自动跟随）
- DNAT 用 `rdr on $if $af proto X from any to any port $port -> $target port $tport`
- BINAT 用 `binat on $if $af from $ip to any -> $ext_ip`
- 协议 `Both` 生成 `proto { tcp udp }`

### NAT 自动放行（auto-pass）

**问题**：whitelist 模式下默认 `block all`，jail 子网的入站流量在 NAT 生效前就被拦掉。用户加完 SNAT 规则后还得手动去「规则管理」加 `pass in quick from <jail subnet>`，否则 NAT 不工作——很容易踩坑。

**方案**：`generate_pf_nat_pass` / `generate_ipfw_nat_pass` 在 whitelist 模式下为每条启用的 NAT 规则**自动注入**配套的过滤放行规则。用户只需添加 NAT 规则即可，无需手动加 pass。

**PF 自动放行**（`generate_pf_nat_pass`）— 在用户过滤规则**之后**嵌入（PF first-match-quick，用户 `block quick` 可覆盖）：

```pf
# --- NAT auto-pass (whitelist mode) ---
# [auto] SNAT pass-in: NAT for jail network
pass in quick inet from 10.0.0.0/24 to any keep state
# [auto] DNAT pass-in: Forward HTTP
pass in quick on em0 inet proto tcp from any to 10.0.0.2 port 8080 flags any keep state (sloppy)
```

- SNAT：放行源网段的入站（不限内部接口，避免猜桥接名）
- DNAT：rdr 已经在过滤前转换了目的地址，所以匹配**内部目标** `dst_addr:dst_port`（而非外部端口）
- BINAT：`pass quick` 双向放行

**ipfw 自动放行**（`generate_ipfw_nat_pass`）— 规则编号 `60000+`，在 NAT 规则（50000+）之后、默认策略（65000/65534）之前：

```sh
# --- NAT auto-pass (whitelist mode) ---
# [60000] [auto] SNAT pass-in: NAT for jail network
ipfw -q add 60000 allow ip from 10.0.0.0/24 to any in keep-state
# [60000] [auto] DNAT pass-in: Forward HTTP
ipfw -q add 60000 allow tcp from any to 10.0.0.2 8080 in recv em0 keep-state
```

**仅在 whitelist 模式生成**——blacklist 模式默认就是 `pass`，无需注入。无启用 NAT 规则时不生成。

**ipfw** — 由两个函数生成：
- `generate_ipfw_nat_config`：`nat N config` 实例声明（在使用 `nat N` 的规则之前）
- `generate_ipfw_nat_rules`：`ipfw -q add N nat N ...` 规则（在过滤规则之后、默认策略之前）

```sh
# --- NAT configuration (one instance per interface, merged) ---
# [NAT 1] NAT for jail network, Forward HTTP
ipfw -q nat 1 config if em0 same_ports reset redirect 10.0.0.2:8080 tcp

# --- Managed Filter Rules ---
ipfw -q add 00100 allow tcp from any to any dst-port 80 in

# --- NAT rules (one via rule per interface) ---
# [NAT 1] via em0
ipfw -q add 50000 nat 1 ip from any to any via em0

# --- NAT auto-pass (whitelist mode, no keep-state) ---
# [60000] [auto] SNAT pass-in: NAT for jail network
ipfw -q add 60000 allow ip from 10.0.0.0/24 to any in
# [60001] [auto] SNAT pass-out: NAT for jail network
ipfw -q add 60001 allow ip from any to 10.0.0.0/24 out
# [60100] [auto] DNAT pass-in: Forward HTTP
ipfw -q add 60100 allow tcp from any to 10.0.0.2 8080 in
# [60101] [auto] DNAT pass-out: Forward HTTP
ipfw -q add 60101 allow tcp from 10.0.0.2 8080 to any out

# [65000] Allow all outbound (whitelist mode)
ipfw -q add 65000 allow ip from any to any out keep-state
# [65534] Default deny
ipfw -q add 65534 deny ip from any to any
```

**关键点**：
- **按接口分组**：同一接口上的所有 NAT 规则（SNAT + DNAT + BINAT）合并为一个 nat 实例。`one_pass=1` 时只有第一条匹配的 nat 规则生效，多实例同接口会有规则不可达
- **`via` 关键字**：一条 `nat N ip from any to any via $iface` 同时匹配进出两个方向。libalias 内部根据包方向自动调用 LibAliasIn（入站 de-NAT）或 LibAliasOut（出站 SNAT）。这与 `/etc/rc.firewall` 的标准做法一致
- `net.inet.ip.fw.one_pass=1`（默认）：`nat` 翻译后包退出防火墙，不继续走后续规则
- 规则编号从 `50000` 开始，步进 `100`（与过滤规则 `00100-49900` 隔离）
- SNAT 用 `same_ports reset`（避免端口冲突，类似 MASQUERADE）
- DNAT 在 nat config 中用 `redirect` 子句，同一实例可配多个 redirect
- 自动放行规则编号 `60000+`，**无 `keep-state`**（见下节说明）

### 生成器签名变更（破坏性）

`generate_pf` / `generate_ipfw` / `preview_config` / `write_config_only` / `apply_pf` / `apply_ipfw` / `init_pf` / `init_ipfw` 均增加 `nat_rules: &[NatRule]` 参数。所有调用方（handlers/firewall.rs）已同步更新。

### ipfw_nat 模块加载

`apply_ipfw` 时若 NAT 规则非空，先调用 `ensure_ipfw_nat()` 加载 `ipfw_nat.ko` 内核模块：
- `ipfw_nat_loaded()` — `kldstat -q -n ipfw_nat` 检查
- `ensure_ipfw_nat()` — 若未加载则 `kldload ipfw_nat`
- 加载失败返回明确错误，不静默应用（避免过滤规则生效但 NAT 缺失）

### Staging 集成

`StagingData` 增加 `nat_rules` 字段（`#[serde(default)]` 向后兼容旧 staging 文件）：

```rust
struct StagingData {
    rules: Vec<FirewallRule>,
    tables: Vec<IpTable>,
    #[serde(default)]
    nat_rules: Vec<NatRule>,    // 旧文件反序列化为空 Vec
}
```

- `write_staging` / `read_staging` 签名增加 NAT 规则参数/返回值
- `effective_state` 返回 `(rules, tables, nat_rules)` 三元组
- `confirm` 时 `replace_all_nat_rules` 与 `replace_all_rules` / `replace_all_tables` 一同提交到 DB（同一事务）
- `rollback` / `discard` 无需改动——`backup_config` 是配置文件全文快照，NAT 段自动包含

### 输入校验（`validate_nat_body`）

| 字段 | 规则 |
|---|---|
| `interface` | 必填，匹配 `^[a-zA-Z0-9_.]{1,15}$` |
| `src_addr` | 必填；`"any"` 或合法 IPv4/IPv6/CIDR |
| `dst_addr` | 可选；非空时校验 IP/CIDR |
| `src_port` / `dst_port` | 匹配 `^(\d+)(-(\d+))?(,(\d+)(-(\d+))?)*$`，1-65535 |
| `kind=dnat` | `src_port` 必填（原端口）；`dst_addr` 必填（内部目标） |
| `kind=binat` | `dst_addr` 必填（外部地址） |
| `description` | ≤ 200 字符，无换行 |

### CRUD 行为

与过滤规则一致，按防火墙启用状态分流：

| 操作 | 防火墙未启用 | 防火墙已启用 |
|---|---|---|
| create_nat_rule | DB INSERT + regen_config | staging 新增 |
| update_nat_rule | DB UPDATE + regen_config | staging 修改 |
| delete_nat_rule | DB DELETE + regen_config | staging 删除 |
| toggle_nat_rule | DB UPDATE + regen_config | staging 修改 enabled |
| reorder_nat_rules | DB UPDATE position + regen_config | staging 修改 position |

> **DB 锁注意事项**：与过滤规则相同——`regen_config()` 内部 `state.db.lock().await`，调用方必须先释放自己的 DB 锁（块作用域 `{ let conn = state.db.lock().await; ... }`）。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/firewall/nat/rules` | 列出所有 NAT 规则（按 position 升序） |
| POST | `/api/firewall/nat/rules` | 添加 NAT 规则 |
| PUT | `/api/firewall/nat/rules/{id}` | 修改 NAT 规则 |
| DELETE | `/api/firewall/nat/rules/{id}` | 删除 NAT 规则 |
| PUT | `/api/firewall/nat/rules/{id}/toggle` | 启用/禁用 |
| PUT | `/api/firewall/nat/rules/reorder` | 重排序 `{ ordered_ids: [...] }` |

> 配置预览、apply、confirm、rollback 复用现有 `/api/firewall/config`、`/api/firewall/apply` 等（自动包含 NAT 段）。

**请求示例**：

```json
// SNAT（为 jail 网段提供出口）
POST /api/firewall/nat/rules
{
    "kind": "snat", "family": "ip", "interface": "em0",
    "src_addr": "10.0.0.0/24", "protocol": "both",
    "description": "NAT for jail network"
}

// DNAT（把主机 80 转发到 jail 10.0.0.2:8080）
POST /api/firewall/nat/rules
{
    "kind": "dnat", "family": "ip", "interface": "em0",
    "src_addr": "any", "dst_addr": "10.0.0.2",
    "src_port": "80", "dst_port": "8080", "protocol": "tcp",
    "description": "Forward HTTP to jail web-01"
}
```

## 外部依赖

- `/sbin/ipfw` — ipfw NAT 规则（`nat N config` + `nat N` 规则）
- `/sbin/pfctl` — pf NAT 规则（随 `pfctl -f` 一同加载）
- `/sbin/kldload` — 加载 `ipfw_nat` 内核模块（ipfw 驱动 + NAT 规则存在时）
- `/sbin/kldstat` — 检查 `ipfw_nat` 模块状态

## 前端

### 文件结构

```
frontend/src/
├── pages/
│   └── FirewallNatPage.vue    # NAT 规则管理（列表 + 编辑表单）
├── lib/menu.js                # /firewall 子菜单加 NAT 项
├── router/index.js            # 路由 /firewall/nat
└── i18n/translations.js       # nav.firewallNat + firewall.nat* 翻译键
```

### 页面功能

- **状态卡片**：显示当前驱动、防火墙状态、规则数；ipfw 驱动时提示 `ipfw_nat` 模块依赖
- **规则列表**：表格列 `# / 启用 / 类型 / 协议 / 接口 / 地址族 / 描述 / 操作`
  - 类型 badge：SNAT=`badge-dim`，DNAT=`badge-success`，BINAT=`badge-warn`
  - 描述为空时显示规则摘要（`src_addr → target` 或 `port X → target:Y`）
- **编辑表单**（`useFormModal` + `DialogHost.vue` form 类型）：
  - 类型/地址族：`type: 'radio'`（radio-pill 样式）
  - 协议/接口：`type: 'select'`，`row: 'proto-iface'` 同行
  - DNAT 时动态显示原端口 + 目标端口（`row: 'dnat-ports'` 同行）
  - `submitHandler` 模式：API 失败时错误内联显示在弹窗内，不丢失输入
- **外部接口自动检测**：页面加载时调用 `/api/network/gateway` 获取默认路由出口，作为接口字段默认值；`/api/network/interfaces` 提供下拉选项
- **pending_apply 提示**：staging 存在时顶部显示警告 + 跳转到规则页应用

### 消息反馈

遵循项目约定（成功 → toast，失败 → 弹窗）：
- 规则添加/编辑/删除成功 → `useToast()`（提示"点击应用变更以生效"）
- API 校验失败 → 表单内 errorMessage（弹窗保持打开）
- 其他操作失败 → `useAlert()`

## 已知限制 / TODO

1. **BINAT 不完整** — 数据模型已定义 `NatKind::Binat`，但 ipfw 路径仅生成 SNAT 侧配置（`nat N config if ... same_ports reset`），缺少完整的双向 1:1 映射模拟。PF 路径已完整支持。完整 BINAT 为 P2 增强
2. **无端口冲突检测** — 同一接口同一端口的 DNAT 规则重复时不报错（pf 用首条匹配，ipfw 用规则号顺序）
3. **无 NAT 状态查看** — 不展示活跃 NAT 连接（`pfctl -s state` / `ipfw nat show`），P2 增强
4. **无 NAT 命中统计** — 不展示每条 NAT 规则的包数/字节数（`pfctl -v -sn`），P2 增强
5. **无 Jail/Bhyve 联动** — 暂无"为该 jail 配置端口转发"的快捷入口，P2 增强
6. **IPv6 NAT66 用例少** — 保留 `family=ip6` 选项供特殊场景，但 NAT66 在实践中较少使用（IPv6 通常全局可达）
7. **外部接口变更未自动跟随** — 若默认路由接口变更（如 DHCP 切换），NAT 规则中的 `interface` 字段不自动更新，需用户手动修改
