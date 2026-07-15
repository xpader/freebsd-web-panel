# 27 — 防火墙（双驱动 ipfw / pf）

## 概述

在 Web 面板中提供统一的防火墙管理界面，支持 FreeBSD 两大防火墙引擎 ipfw 和 pf。用户可选择驱动、切换驱动、以结构化表单增删改规则、切换黑白名单模式。规则统一存储在 SQLite 中，切换引擎时自动生成对应语法的配置文件。

**关键设计决策**：
- 规则存储与引擎解耦——一套规则在 DB 中，切换引擎时重新生成对应语法的 `/etc/ipfw.rules` 或 `/etc/pf.conf`
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
enum AddressKind { Any, Single, Cidr, Me }

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
    id, driver, position, enabled,
    action, direction, protocol,
    src_kind, src_value, src_port,
    dst_kind, dst_value, dst_port,
    interface, log, icmp_type, description,
    created_at, updated_at
);

CREATE TABLE firewall_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- keys: 'active_driver' → 'ipfw'|'pf'
--       'mode'          → 'whitelist'|'blacklist'
--       'rules_dirty'   → '1'|'0'
```

> **注意**：`driver` 列保留但不再用于过滤——所有规则统一存储，切换引擎时共用同一套规则。

### 规则生成

**ipfw**（`generate_ipfw`）— 生成 shell 脚本 `/etc/ipfw.rules`：
- 规则编号从 00100 开始，步进 100
- 白名单模式末尾加 `ipfw -q add 65534 deny ip from any to any`
- 黑名单模式末尾加 `ipfw -q add 65534 allow ip from any to any`
- allow 规则自动加 `keep-state`
- ICMP 类型用数字（ipfw 不接受名字）
- 离散/连续端口转换：`80,443,8080-8090` → `{ 80 443 8080-8090 }`

**pf**（`generate_pf`）— 生成 `/etc/pf.conf`：
- 白名单模式：`block all` + `pass out quick all keep state` + `set skip on lo0`
- 黑名单模式：`set skip on lo0`（pf 默认放行）
- 用户规则使用 `quick`（首个匹配生效，与 ipfw 语义一致）
- `set skip on lo0` 必须在 `block all` 前面
- allow 规则加 `keep state`（TCP 加 `flags S/SA keep state`）
- ICMP 类型用数字（FreeBSD pf 不接受名字）
- 离散/连续端口转换：`80,443,8080-8090` → `{ 80, 443, 8080:8090 }`

### ICMP 类型处理

前端下拉框显示友好名称（如 `echo-request (8) Ping`），存储和生成时用数字。`icmp_name_to_number()` 函数将名称映射为数字。

### 端口格式转换

用户输入统一格式 `80,443,8080-8090`（逗号分隔，短横线表示范围），生成器转换为各引擎语法：
- ipfw: `port_to_ipfw()` → `{ 80 443 8080-8090 }`
- pf: `port_to_pf()` → `{ 80, 443, 8080:8090 }`（pf 范围用冒号）

### 初始化流程

`init_ipfw(mode, rules)`：
1. sysrc 设置 `firewall_enable=YES`、`firewall_script=/etc/ipfw.rules`、`firewall_logging=YES`
2. sysrc 删除 `firewall_type`（避免与 script 冲突）
3. `kldload ipfw`（如未加载）
4. 生成配置文件（`atomic_write` 到 `/etc/ipfw.rules`）
5. **不加载规则、不启用防火墙**——用户手动 Apply + Enable

`init_pf(mode, rules)`：
1. sysrc 设置 `pf_enable=YES`、`pf_rules=/etc/pf.conf`
2. `kldload pf`（如未加载）
3. 生成配置文件
4. **不加载规则、不启用防火墙**

### 启用/禁用

**enable**：确保当前驱动 `*_enable=YES`，对方 `*_enable=NO`，然后运行时启用。
- ipfw: `sysctl net.inet.ip.fw.enable=1`
- pf: `pfctl -e`

**disable**：运行时禁用 + rc.conf 设为 `NO`。
- ipfw: `sysctl net.inet.ip.fw.enable=0` + `sysrc firewall_enable=NO`
- pf: `pfctl -d`（fire-and-forget，pf 未启用时不报错）+ `sysrc pf_enable=NO`

### 切换引擎

1. 禁用旧驱动（`deactivate_ipfw`/`deactivate_pf`）
2. 初始化新驱动（`init_*`）
3. 加载新驱动规则（`apply_*`）
4. 更新 DB `active_driver`
5. 不自动启用——返回 `enabled=false`

### 切换黑白名单模式

1. 更新 DB `mode`
2. 重新 apply 规则（ipfw 和 pf 都需要重新生成+加载）
3. ipfw 模式切换不写 loader.conf/sysctl.conf——通过规则 65534 实现

### Apply 流程

1. 从 DB 读取所有 enabled 规则
2. 调用生成器生成配置文件内容
3. 原子写入配置文件（临时文件 → rename）
4. 加载规则（ipfw: `sh /etc/ipfw.rules`；pf: `pfctl -f /etc/pf.conf`）
5. 设 `rules_dirty=0`

### 文件结构

```
src/
├── firewall_gen.rs          # 类型定义、DB CRUD、配置生成、驱动操作
├── handlers/firewall.rs     # API handler 函数
├── db.rs                    # firewall_rules / firewall_state 表创建（migrate 函数内）
└── app.rs                   # 路由注册

frontend/src/
├── pages/PfPage.vue         # 防火墙页面（状态卡片 + 规则列表 + 配置预览）
└── components/ui/DialogHost.vue  # 修复了 showIf/requiredIf 支持 select 字段 + 数组匹配
```

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
| POST | `/api/firewall/apply` | 生成配置 + 加载规则 |
| GET | `/api/firewall/config` | 预览生成的配置文件内容 |

## 外部依赖

- `/sbin/ipfw` — ipfw 规则管理
- `/sbin/pfctl` — pf 规则管理
- `/sbin/kldload` — 加载内核模块
- `/sbin/kldstat` — 检查模块是否已加载
- `/sbin/sysctl` — 运行时参数设置
- `/usr/sbin/sysrc` — rc.conf 读写（复用 `sysrc.rs` 模块）
- `/bin/sh` — 执行 ipfw 规则脚本

## 配置项

无专用 `fwp.toml` 配置项。防火墙状态存储在 SQLite `firewall_state` 表中。

## 已知限制 / TODO

1. **不支持 NAT/转发规则** — 仅过滤规则
2. **不支持 pf tables** — 仅静态地址
3. **不导入已有配置** — 初始化时生成空白规则集（仅默认策略）
4. **pf 的 `(self)` 地址** — 表示本机所有 IP，不支持指定接口地址
5. **无规则可达性分析** — 不检测规则冲突或冗余
6. **ipfw 的 `default_to_accept` boot tunable** — 不修改 loader.conf，通过规则 65534 在运行时控制
