# 设计：监控（Monitoring）

> 功能计划文档。**实现前需用户确认。**

## 1. 目标

为系统提供持续的资源与服务监控能力，区别于仪表盘的"当前快照"：
- **时序记录** — 周期性采样并持久化历史指标（CPU/内存/磁盘/网络/温度）
- **图表展示** — 折线/面积图，支持时间范围选择（1h/6h/24h/7d/30d）
- **告警规则** — 阈值触发通知（CPU>80%、磁盘>90%、温度>75°C、服务停止等）
- **通知渠道** — 邮件、Webhook（后续可扩展 Telegram/钉钉）

## 2. 与仪表盘的关系

| | 仪表盘（已完成） | 监控（本模块） |
|---|---|---|
| 数据 | 实时快照（每次请求采样） | 历史时序（周期采样+存储） |
| 展示 | 数值 + 进度条 | 时间序列折线图 |
| 时间范围 | 仅当前 | 1h ~ 30d 可选 |
| 告警 | 无 | 阈值规则 + 通知 |
| 存储 | 无（内存瞬时值） | SQLite（或可选 InfluxDB） |

## 3. 架构

```
┌─────────────────────────────────────────────────┐
│  fwp 主进程 (axum)                               │
│  ├── 采集器 (tokio 后台任务, 每 N 秒)             │
│  │     读 sysctl/swapinfo/ifconfig/zpool         │
│  │     → 写 SQLite samples 表                    │
│  ├── 查询 API (/api/monitor/*)                   │
│  └── 告警引擎 (检查规则 → 触发通知)              │
│                                                  │
│  前端 /#/monitor                                 │
│    图表(Chart) + 规则配置 + 告警历史             │
└─────────────────────────────────────────────────┘
```

### 3.1 数据存储（SQLite）

轻量优先，单机面板不引入额外数据库。表设计：

```sql
-- 周期采样（每 30s 一条，约 2880 条/天）
CREATE TABLE metric_samples (
    ts          INTEGER NOT NULL,   -- unix 秒
    category    TEXT NOT NULL,      -- 'cpu'|'memory'|'swap'|'disk'|'net'|'temp'
    name        TEXT NOT NULL,      -- 'cpu.total'|'cpu.0'|'mem.used'|'temp.cpu0'...
    value       REAL NOT NULL,
    PRIMARY KEY (ts, category, name)
);
CREATE INDEX idx_samples_cat ON metric_samples(category, name, ts);

-- 自动轮转：保留 30 天，超期删除（每日清理任务）

-- 告警规则
CREATE TABLE alert_rules (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    metric      TEXT NOT NULL,      -- 'cpu.total'|'memory.usage'|'temp.max'...
    operator    TEXT NOT NULL,      -- '>'|'<'|'>='|'<='
    threshold   REAL NOT NULL,
    duration_sec INTEGER NOT NULL DEFAULT 60,  -- 持续 N 秒才触发
    enabled     INTEGER NOT NULL DEFAULT 1,
    severity    TEXT DEFAULT 'warn', -- 'warn'|'critical'
    created_at  INTEGER NOT NULL
);

-- 告警事件（触发记录）
CREATE TABLE alert_events (
    id          INTEGER PRIMARY KEY,
    rule_id     INTEGER NOT NULL,
    ts          INTEGER NOT NULL,
    value       REAL NOT NULL,
    resolved    INTEGER DEFAULT 0,
    resolved_ts INTEGER,
    message     TEXT
);
```

### 3.2 采样间隔与保留策略

| 粒度 | 间隔 | 保留 |
|---|---|---|
| 原始 | 30s | 7 天 |
| 降采样 1m | 1 分钟聚合 | 30 天 |
| 降采样 10m | 10 分钟聚合 | 1 年（可选） |

初版实现原始 30s + 30 天保留；降采样作为优化后续加入。

### 3.3 采集指标清单

| 类别 | 指标 | 来源 |
|---|---|---|
| **cpu** | `cpu.total`（总体使用率%）、`cpu.N`（每核%） | `kern.cp_times`（复用仪表盘 delta 逻辑） |
| **memory** | `mem.used`、`mem.usage`（%）、`mem.wired` | `vm.stats.vm.*` |
| **swap** | `swap.used`、`swap.usage`（%） | `swapinfo` |
| **disk** | `disk.<pool>.used`、`disk.<pool>.usage`（%） | `zfs list` / `zpool list` |
| **net** | `net.<if>.rx_bytes`、`net.<if>.tx_bytes`、`net.<if>.rx_rate`、`net.<if>.tx_rate` | `ifconfig` / `netstat -ib` |
| **temp** | `temp.cpuN`（每核 °C）、`temp.max` | `dev.cpu.N.temperature` |
| **load** | `load.1`、`load.5`、`load.15` | `uptime` |

## 4. 前端设计

### 4.1 页面结构（顶部主菜单「监控」）

| 左侧子菜单 | 内容 |
|---|---|
| **概览** | 所有图表的紧凑总览（CPU/内存/磁盘/网络小图） |
| **CPU** | 总体 + 每核历史折线图 |
| **内存** | 内存/Swap 使用历史 |
| **磁盘** | 各 ZFS pool 使用率 + IO |
| **网络** | 各接口流量历史（rx/tx 速率） |
| **温度** | 各核心温度历史 |
| **告警规则** | 规则 CRUD（metric/op/threshold/duration） |
| **告警历史** | 触发记录列表（含已恢复/未恢复） |

### 4.2 图表

- **不引入重型图表库**：用轻量 `<canvas>` 自写折线图绘制器（约 150 行），或用 `Chart.js`（ESM 单文件本地副本，~70KB）
- 倾向 Chart.js：成熟、交互（tooltip/缩放）完善，本地托管避免 CDN 依赖
- 时间范围选择器：1h / 6h / 24h / 7d / 30d 按钮

### 4.3 菜单集成

顶部主菜单新增「监控」标签（位于「系统」左侧）：
```
概览  配置  网络  文件系统  虚拟化  监控  系统
```

## 5. API 设计

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/monitor/series` | 查询时序数据（`?category=&name=&from=&to=`） |
| GET | `/api/monitor/latest` | 各指标最新值（概览页用） |
| GET | `/api/monitor/rules` | 列出告警规则 |
| POST | `/api/monitor/rules` | 创建规则 |
| PUT | `/api/monitor/rules/:id` | 修改规则 |
| DELETE | `/api/monitor/rules/:id` | 删除规则 |
| GET | `/api/monitor/events` | 告警事件历史（`?resolved=&limit=`） |
| POST | `/api/monitor/events/:id/resolve` | 手动标记已处理 |

## 6. 通知渠道

初版：
- **Webhook**（POST JSON 到配置的 URL，通用，可接任意系统）
- **邮件**（SMTP，复用系统 `sendmail` 或直接 SMTP 连接）

配置在 `fwp.toml`：
```toml
[monitor]
enabled = true
interval_sec = 30
retention_days = 30

[monitor.notify.webhook]
enabled = false
url = "https://example.com/hook"

[monitor.notify.smtp]
enabled = false
host = "smtp.example.com"
port = 587
from = "fwp@example.com"
to = ["admin@example.com"]
```

## 7. 实现里程碑

| 阶段 | 内容 |
|---|---|
| **M1** | 采集器后台任务 + SQLite 存储表 + 基础指标采样（CPU/内存/Swap/温度/负载） |
| **M2** | `/api/monitor/series` + `/api/monitor/latest` 查询 API |
| **M3** | 前端监控概览页 + 图表组件（Chart.js 本地托管） |
| **M4** | 分类详情页（CPU/内存/磁盘/网络/温度） |
| **M5** | 告警规则 CRUD + 告警引擎（持续阈值检测） |
| **M6** | 告警历史页 + 手动处理 |
| **M7** | 通知渠道（Webhook + SMTP 邮件） |
| **M8** | 数据轮转清理 + 降采样优化 |

## 8. 风险与缓解

| 风险 | 缓解 |
|---|---|
| SQLite 高频写入性能 | 30s 间隔写入量极低（每天 ~10MB）；WAL 模式；批量插入 |
| 长期数据膨胀 | 自动轮转删除 + 后续降采样聚合 |
| 图表库体积 | Chart.js ESM 单文件 ~70KB，本地托管，可接受 |
| 采集影响系统性能 | 仅读 sysctl（零开销）+ 低频；避免 spawn 过多子进程 |
| 告警风暴（持续触发） | duration_sec 持续窗口 + 事件去重（同一规则未恢复不重复触发） |
