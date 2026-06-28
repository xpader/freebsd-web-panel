
## 1. 目标

构建一个原生运行于 FreeBSD 上的 Web 系统管理面板，通过浏览器管理:

- **系统配置** — sysctl、rc.conf、网络接口、防火墙 (pf/ipfw)、服务 (rc.d)
- **Jail 容器** — 直接通过 `libjail` (jail_set/jail_get/jail_remove 系统调用) 管理，**不依赖任何第三方 jail 工具**
- **Bhyve 虚拟机** — 通过 `vm-bhyve` CLI 命令封装管理
- **ZFS 文件系统** — 通过 `zfs`/`zpool` 命令封装管理

## 2. 运行环境（已确认）

| 项目 | 版本 / 路径 |
|---|---|
| OS | FreeBSD 15.1-RELEASE (amd64) |
| jail(8) | 系统自带 `/usr/sbin/jail` |
| libjail | `/lib/libjail.so.1` + `/usr/include/jail.h` |
| vm-bhyve | 1.7.3 (`/usr/local/sbin/vm`)，已有运行中 VM |
| zfs/zpool | `/sbin/zfs`、`/sbin/zpool`，pool `zroot` |
| sysrc | `/usr/sbin/sysrc` |
| ifconfig | `/sbin/ifconfig` |
| pfctl | `/sbin/pfctl` |
| 工具链 | Rust 1.94 / Node 24 / Python 3.11 |

## 3. 技术架构决策

```
┌─────────────────────────────────────────────────────────┐
│                    浏览器 (SPA 前端)                     │
│              纯 HTML/CSS/JS，Axum 静态托管               │
└────────────────────────┬────────────────────────────────┘
                         │ HTTPS + Token
┌────────────────────────▼────────────────────────────────┐
│                  Rust 后端 (fwp 单二进制)                │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────┐ │
│  │ axum 路由 │  │ 认证中间件│  │ 任务队列  │  │审计日志 │ │
│  └────┬─────┘  └──────────┘  └────┬─────┘  └─────────┘ │
│       │                            │                     │
│  ┌────▼────────────────────────────▼─────────────────┐  │
│  │              Module 层 (业务逻辑)                  │  │
│  │  sysctl │ rcconf │ net │ pf │ jail │ bhyve │ zfs  │  │
│  └────┬─────────┬──────┬─────┬─────┬──────┬─────┬────┘  │
│       │         │      │     │     │      │     │       │
│  ┌────▼───┐ ┌───▼──┐ ┌─▼──┐ ┌▼──┐ ┌▼───┐ ┌▼──┐ ┌▼───┐   │
│  │sysctl  │ │sysrc │ │ifc │ │pfc│ │libjail│ │vm │ │zfs│   │
│  │ FFI?   │ │子进程│ │子进程│ │子进程│ │FFI│ │子进程│ │子进程│  │
│  └────────┘ └──────┘ └────┘ └───┘ └───┘ └───┘ └───┘   │
└─────────────────────────────────────────────────────────┘
```

### 3.1 后端：Rust + Axum

**理由：**
- **单二进制部署** — 编译产出唯一可执行文件，符合系统管理工具特性
- **FFI 能力** — jail 模块需要直接调用 `libjail` 的 C API，Rust FFI 成熟安全
- **性能** — 系统面板可能在高负载服务器上运行，零开销抽象
- **强类型** — 配置项、JSON Schema、命令输出解析都能编译期保证
- **生态** — tokio 异步运行时、axum Web 框架、serde 序列化均生产可用

**核心依赖：**
- `tokio` (full) — 异步运行时
- `axum` 0.8 — HTTP 框架，带宏路由、WebSocket
- `tower-http` — 静态文件、CORS、日志中间件
- `serde` / `serde_json` — API 数据序列化
- `libc` + 原生 FFI — 调用 libjail
- `clap` — 命令行参数
- `tracing` — 结构化日志

### 3.2 前端：原生 SPA（无构建步骤）

**理由：**
- 避免引入 Node 构建链，系统管理面板不需要重型前端工程
- 用 ES Modules + `<script type="module">` 组织代码
- 样式用原生 CSS + 轻量组件库（Pico.css 或自写）
- 由 Axum `ServeDir` 直接托管，部署即一个二进制 + 静态目录

> 详见 `50-frontend.md`。若后期 UI 复杂度上升，可在不改动后端 API 的前提下迁移到 Vue/React。

### 3.3 Jail 实现策略（关键约束）

**需求：不依赖任何外部 jail 工具（不用 iocage/ezjail/jailmanager 等）。**

实现方式：
1. **配置持久化** — 解析/生成 `/etc/jail.conf`（jail.conf(5) 语法，含变量替换 `+/%` 操作符、引号、注释）
2. **运行时控制** — 直接 FFI 调用 `libjail.so.1`：
   - `jailparam_all()` — 枚举所有可用参数
   - `jailparam_get()` — 读取运行中 jail 的参数（替代 `jls`）
   - `jailparam_set()` — 创建/修改 jail（替代 `jail -c`/`jail -m`）
   - `jail_remove()` — 删除 jail（替代 `jail -r`）
3. **jail.conf 解析器** — 自写递归下降解析器（语法在 jail.conf(5) 文档中明确），保留注释和格式

> 详见 `20-jail.md`。

### 3.4 Bhyve 实现策略

通过子进程执行 `vm` 命令，解析其文本表格输出。已确认 vm-bhyve 1.7.3 已安装并有运行中 VM。
封装全部常用命令：list/create/start/stop/install/console/snapshot/clone/iso 等。

> 详见 `30-bhyve.md`。

### 3.5 ZFS 实现策略

通过子进程执行 `zfs`/`zpool`，配合 `-H -o` 机器可读输出格式（tab 分隔），稳定解析。
覆盖：pool 状态、dataset CRUD、快照、发送/接收、属性管理。

> 详见 `40-zfs.md`。

## 4. 部署形态

- **单二进制** `fwp`，监听本地端口（默认 `127.0.0.1:8443`）
- 配置文件 `/usr/local/etc/fwp.toml`
- 数据目录 `/var/db/fwp/`（会话、审计日志、token）
- **以 root 运行**（系统管理本质要求）；通过 systemd/rc.d 脚本或 `daemon` 启动
- 提供 rc.d 集成脚本 `fwp_enable=YES`
- 前端资源内嵌二进制（`include_dir!` 或编译期嵌入）或独立 `share/fwp/web/` 目录

## 5. 安全模型

- 默认仅监听 `127.0.0.1`，远程访问需显式配置或前置反向代理
- Token-based 认证（HMAC-SHA256），初始 token 首次启动生成
- 可选 PAM 认证（FreeBSD `pam_unix`）映射系统用户
- 全部写操作记录审计日志（who/when/what/result）
- HTTPS（自签名证书自动生成）或 HTTP 模式（反代场景）

> 详见 `60-security-auth.md`。

## 6. 非目标（明确排除）

- ❌ 多节点/集群管理（单机面板）
- ❌ 用户多租户/RBAC（单管理员模型；后续可扩展）
- ❌ 容器编排（不是 Kubernetes 替代品）
- ❌ 基于 FreeBSD ports 的软件包商店（用 `pkg` 即可）
- ❌ 监控/告警系统（可用现有 prometheus/grafana）
