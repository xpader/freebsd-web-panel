# FreeBSD Web Panel

[English](README.md) | [中文](README.zh-CN.md)

一个基于 Web 的 FreeBSD 系统管理面板。通过浏览器管理 sysctl、rc.conf、网络、服务、PF 防火墙、Jail 容器、Bhyve 虚拟机、ZFS 文件系统等——全部集成在一个自带 Web UI 的单二进制文件中。

> 目标平台：**FreeBSD 15.x amd64**。以 root 运行。

![截图](screenshot.png)

## 功能特性

| 模块 | 能力 |
|---|---|
| **仪表盘** | 通过 sysctl 实时获取 CPU、内存、负载、温度指标 |
| **监控** | 时序图表（Chart.js），可配置采样间隔与数据保留周期 |
| **Sysctl** | 浏览、搜索、修改运行时参数，持久化到 `sysctl.conf` |
| **rc.conf** | 通过 `sysrc` 进行完整增删改查，含分类描述 |
| **定时任务** | 管理系统用户的 crontab |
| **服务** | 列出、启动、停止、重启 rc.d 服务 |
| **网络** | 网卡接口详情、路由表、默认网关、DNS 名称服务器 |
| **系统账户** | 浏览 FreeBSD 用户与用户组 |
| **文件管理器** | 浏览、上传、下载、重命名、修改权限/属主 |
| **ZFS** | 存储池状态/scrub、数据集增删改查、快照、回滚、克隆 |
| **Jail 容器** | 通过原生 libjail FFI 实现完整生命周期管理——**不依赖任何第三方 jail 工具**（jail.conf 解析器 + 创建/启动/停止/删除、基础镜像管理） |
| **Web 终端** | 基于 WebSocket 的浏览器内 Shell 访问 |
| **用户与认证** | 自带用户体系（Argon2id 密码哈希）、会话令牌、首启引导 |
| **审计日志** | 所有写操作均记录（谁/何时/做了什么/结果） |
| **国际化** | 多语言界面（中文、英文），运行时切换 |

> **规划中：** PF 防火墙编辑器、Bhyve（vm-bhyve）虚拟机管理。

## 技术栈

- **后端：** Rust 2021（MSRV 1.74）、Axum 0.8、tokio、rusqlite（内嵌 SQLite）、argon2、rust-embed
- **前端：** 原生 JS ES Modules，手写深色主题 CSS。**无构建步骤、无框架**——保持部署简单。
- **部署：** 单二进制文件，Web 资源内嵌。TOML 配置位于 `/usr/local/etc/fwp.toml`。
- **Jail FFI：** 直接调用 libjail（`jailparam_*`）——所有 `unsafe` 代码集中在专用 `sys` 子模块中。

## 快速开始

### 前置条件

- FreeBSD 15.x（amd64）
- Rust 工具链（1.74+）
- 系统工具：`sysctl`、`sysrc`、`ifconfig`、`zfs`、`zpool`（均为系统自带）

### 构建

```sh
cargo build --release
```

启用 LTO 并 strip 符号的 release 二进制文件输出到 `target/release/fwp`。

### 运行（开发模式）

```sh
cargo run -- --config fwp.toml
```

使用仓库中的 `fwp.toml`，面板监听 `127.0.0.1:8080`，并从 `web/` 目录提供静态资源（文件改动实时生效——改前端无需重新编译）。

### 首次使用

1. 在浏览器打开 `http://127.0.0.1:8080`。
2. 如果尚无用户，引导页面会引导创建第一个管理员账户（无需认证，仅此一次）。
3. 登录后即可开始管理系统。

## 配置

首次运行时，如果 `/usr/local/etc/fwp.toml` 不存在会自动创建：

```toml
[server]
listen = "127.0.0.1:8080"                  # 监听地址
web_root = "/usr/local/share/fwp/web"      # Web 资源磁盘覆盖路径

[paths]
db = "/var/db/fwp/fwp.db"                  # SQLite 数据库
audit = "/var/db/fwp/audit.log"            # 审计日志

[auth]
session_ttl = 28800                         # 会话有效期（秒）

[monitor]
enabled = true
interval_sec = 30                           # 采样间隔
retention_days = 30                         # 数据保留天数
```

使用 `--config /path/to/fwp.toml` 指定配置文件路径。

## 生产环境部署

### rc.d 服务

安装二进制文件和启动脚本：

```sh
cp target/release/fwp /usr/local/sbin/fwp
cp rc.d/fwp /usr/local/etc/rc.d/fwp
chmod +x /usr/local/etc/rc.d/fwp
```

启用并启动：

```sh
sysrc fwp_enable=YES
service fwp start
```

### 反向代理

面板仅提供 HTTP 服务。远程访问请在前面放置带 TLS 的反向代理（如 nginx、Caddy）：

```
server {
    listen 443 ssl http2;
    server_name panel.example.com;

    ssl_certificate     /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }

    location /api/term/ws {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

## 项目结构

```
src/
├── main.rs           # CLI 入口、配置加载、服务绑定
├── app.rs            # 路由组装
├── state.rs          # AppState（共享状态）
├── config.rs         # TOML 配置结构体 + 加载/创建
├── error.rs          # ApiError → HTTP 响应映射
├── db.rs             # SQLite 打开 + 辅助函数
├── auth.rs           # Argon2 哈希、会话令牌、认证中间件
├── audit.rs          # 追加式 JSON 审计日志
├── monitor.rs        # 后台指标采集器 + 时序查询 API
├── jail.rs           # libjail FFI + jail.conf 解析器
├── terminal.rs       # WebSocket Shell（PTY）
├── sysinfo.rs        # 通过 sysctl 获取系统信息
├── web_assets.rs     # rust-embed + 磁盘回退资源 handler
└── handlers/         # HTTP handler（每模块一个文件）

web/
├── index.html        # SPA 入口
├── css/app.css       # 深色主题样式
└── js/
    ├── main.js       # 应用入口、路由注册
    ├── router.js     # 基于 hash 的路由
    ├── api.js        # fetch 封装（auth header、错误处理）
    ├── i18n/         # 国际化
    ├── ui/           # 布局、通知、确认对话框、模态框
    └── pages/        # 每个页面一个模块

docs/
├── plan/             # 设计文档（目标与架构）
└── impl/             # 实现文档（具体怎么做）
```

## 开发

```sh
# 后端检查
cargo check

# 前端语法检查
node --check web/js/main.js

# 用开发配置运行（Web 资源直接从仓库目录读取）
cargo run -- --config fwp.toml
```

服务器解析静态资源时先尝试磁盘 `web_root`，再回退到内嵌资源——因此改前端无需重新编译即可实时生效。

## 安全

- **默认仅监听 localhost**——远程访问需显式配置或前置反向代理。
- **自带认证：** SQLite 用户表、Argon2id 密码哈希、SHA-256 会话令牌哈希。不依赖 PAM 或系统用户。
- **首启引导：** 无用户时 `/api/users/bootstrap` 创建首个管理员（无需认证，仅一次）。
- **审计追踪：** 所有写操作均被记录。
- **以 root 运行**——系统管理任务的必要要求。

## 文档

- [设计计划](docs/plan/) — 各模块的架构与接口设计
- [实现文档](docs/impl/) — 各功能的具体实现方式，含数据结构与 API
- [路线图](docs/plan/80-roadmap.md) — 分阶段交付计划

## 许可证

[MIT](LICENSE) &copy; 2026 Pader
