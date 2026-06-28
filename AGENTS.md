# AGENTS.md

AI 编码代理在 FreeBSD Web Panel（`fwp`）项目上工作时的指引。

## 项目

一个基于 Web 的 FreeBSD 系统管理面板。管理 sysctl、rc.conf、网络、服务、PF 防火墙、Jail 容器（通过原生 libjail FFI，不依赖任何第三方 jail 工具）、Bhyve 虚拟机（通过 vm-bhyve）、ZFS 文件系统。自带用户体系（非系统用户），HTTP API + SPA 前端，单二进制部署。

目标平台：FreeBSD 15.x amd64。以 root 运行（系统管理需要）。

## 技术栈

- **后端**：Rust 2021 edition（MSRV 1.74）、Axum 0.8、tokio、rusqlite（bundled SQLite）、argon2（密码哈希）、rust-embed（通过 `embed-web` 默认 feature 将 Web 资源嵌入二进制）。
- **前端**：原生 JS ES Modules，手写深色主题 CSS。**无构建步骤、无框架**（刻意为之——保持简单，便于单二进制部署）。
- **配置**：TOML 格式，位于 `/usr/local/etc/fwp.toml`（首次运行自动生成默认配置）。数据位于 `/var/db/fwp/`。

## 构建与运行

```sh
cargo build                  # debug 构建
cargo build --release        # release 构建（LTO、strip）
cargo run -- --config /path/to/fwp.toml   # 用指定配置运行
cargo run -- --config fwp.toml            # 开发配置（见下）
```

### 开发配置（本地测试用）

```toml
# fwp.toml — web_root 指向仓库目录，以便实时反映文件改动
[server]
listen = "127.0.0.1:18080"
web_root = "web"

[paths]
db = "/tmp/fwp-test/fwp.db"
audit = "/tmp/fwp-test/audit.log"

[auth]
session_ttl = 28800
```

服务器解析静态资源时先尝试磁盘 `web_root`，再回退到内嵌资源。开发时把 `web_root` 设为仓库的 `web/` 目录，这样改前端无需重新编译。生产环境内嵌资源可从任意工作目录运行。

### JS 检查

```sh
node --check web/js/main.js              # 语法检查前端模块
```

未配置 linter/构建工具——保持 JS 为可通过 `node --check` 的合法 ES module。

## 代码结构

```
src/
├── main.rs           # 入口：CLI（clap）、配置加载、db/audit 打开、服务绑定
├── state.rs          # AppState（共享状态，抽出独立模块以避免循环依赖）
├── app.rs            # 路由组装 + 回退到 web_assets
├── config.rs         # Config 结构体 + TOML 加载/创建
├── error.rs          # ApiError → HTTP 响应；ApiResult<T> = Result<T, ApiError>
├── db.rs             # SQLite 打开 + 自由函数（user_count、get_user 等）
├── auth.rs           # 密码哈希（argon2）、session token、require_auth 中间件、
│                     # AuthUser 提取器（FromRequestParts）
├── audit.rs          # 追加式 JSON 审计日志（parking_lot::Mutex<File>）
├── monitor.rs        # 监控采集器（后台 tokio 任务）+ 时序查询 API
├── web_assets.rs     # rust-embed + 磁盘回退的资源 handler
└── handlers/
    ├── auth.rs       # login / logout / me
    ├── users.rs      # 用户 CRUD + bootstrap（首启创建管理员）
    ├── system.rs     # 系统信息 + 实时指标（CPU/内存/温度，通过 sysctl）
    ├── audit.rs      # 审计日志读取
    └── mod_stubs.rs  # 未实现模块的占位 handler（返回 "planned"）

web/
├── index.html        # SPA 入口
├── css/app.css       # 全部样式（深色主题、顶部栏+侧边栏布局、指标进度条）
└── js/
    ├── main.js       # 应用入口、路由注册、登出 handler
    ├── router.js     # 基于 hash 的路由
    ├── api.js        # fetch 封装（auth header + token + 错误处理）
    ├── ui/
    │   ├── layout.js     # 两级导航：顶部栏（主菜单）+ 侧边栏（子菜单）
    │   ├── confirm.js    # 基于 Promise 的确认对话框
    │   └── toast.js      # 通知
    └── pages/
        ├── audit.js      # 审计日志查看
        ├── monitor.js    # 监控图表（Chart.js 折线图 + 时间范围）
        └── planned.js    # 模块占位页工厂

vendor/                   # 第三方库本地副本
├── chart.umd.min.js              # Chart.js 4.4.7
└── chartjs-adapter-date-fns.bundle.min.js  # Chart.js 时间轴适配器

docs/plan/                # 设计计划文档（功能要做什么）
docs/impl/                # 实现文档（功能怎么做的，开发/变更时必须维护）
rc.d/fwp                  # FreeBSD rc.d 启动脚本

## 编码约定

### Rust

- **错误处理**：`ApiError`（thiserror）→ `IntoResponse` 映射到 HTTP 状态码。Handler 返回 `ApiResult<T>`，其中 `T: IntoResponse`。不要对可失败操作 `unwrap()`——通过 `?` 传播为 `ApiError`。
- **数据库访问**：`db.rs` 中的 SQLite 自由函数接收 `&Connection`。`Db` 类型为 `Arc<tokio::sync::Mutex<Connection>>`（异步 mutex——guard 在 handler 的 `.await` 间持有）。用 `state.db.lock().await` 加锁，调用自由函数：`db::get_user(&conn, id)`。
- **Mutex 选择**：不跨 `.await` 的同步代码用 `parking_lot::Mutex`（如 `LAST_CP_TIMES`、审计日志文件）。仅当 guard 必须在 `.await` 间存活时才用 `tokio::sync::Mutex`。禁止用 `std::sync::Mutex`。
- **静态变量**：初始化在编译期已知的用 `std::sync::LazyLock`（不用 `once_cell`、不用 `OnceLock`，除非需要运行时输入）。
- **路由**：axum 0.8 捕获参数语法是 `{name}`（不是 `:name`）。
- **FFI**：Jail 模块将使用 libjail（`jailparam_*`）——所有 `unsafe` 集中在 `sys` 子模块，配安全封装。
- **匹配模式**：用 match ergonomics（`match &value`），不在模式里写显式 `ref`/`ref mut`。

### 前端

- **无框架、无构建**：纯 ES module，`<script type="module">`。不要加 import map、bundler、转译器。
- **资源服务**：默认内嵌（rust-embed）；开发时磁盘覆盖。
- **API 调用**：用 `js/api.js` 的 `api.get/post/put/del`（处理 auth header + token + 错误 toast）。token 存 `sessionStorage`。
- **导航**：hash 路由（`#/dashboard`）。布局 = 顶部栏主标签 + 侧边栏子项。菜单结构在 `js/ui/layout.js`。
- **定时器清理**：定时器句柄（setInterval）——直接调用 `clearInterval(handle)`，对 null/undefined 是 no-op；不要加真值判断守卫。

## 系统命令模式

FreeBSD 管理通过 spawn 系统二进制并传校验过的参数完成。**禁止字符串拼接 shell**——始终用 `Command::new().arg()` 防注入。传给命令前先校验输入（如 jailname 匹配 `^[a-zA-Z0-9_.-]+$`）。

本机已确认存在的关键工具：`/sbin/sysctl`、`/usr/sbin/sysrc`、`/sbin/ifconfig`、`/sbin/pfctl`、`/sbin/zfs`、`/sbin/zpool`、`/usr/sbin/jail`、`/usr/local/sbin/vm`（vm-bhyve 1.7.3）。

## 文档维护（强制）

项目维护两套文档，**开发或变更功能时必须同步维护**：

- `docs/plan/` — 设计计划：功能的目标、架构决策、接口设计（实现前写的，也实现后的前瞻规划）
- `docs/impl/` — 实现文档：功能实际怎么做的，含数据结构、算法、调用链、API、配置项、已知限制

### 规则

1. **开发新功能前**：先读 `docs/impl/` 中相关的已有实现文档，复用已有模式和约定。`docs/impl/README.md` 有索引。
2. **实现新功能后**：在 `docs/impl/` 创建对应实现文档（编号续接），遵循 README 中的格式规范。
3. **变更已有功能时**：更新对应的 `docs/impl/` 文档，保持与代码一致。
4. **删除功能时**：删除或归档对应文档。
5. **设计阶段**（未实现的功能规划）：写 `docs/plan/`；**实现完成后**：写 `docs/impl/`。

不要让文档与代码脱节——过时的文档比没有文档更危险。

## 架构决策

- **纯 HTTP**（无 TLS）——远程访问请前置反向代理。
- **自带认证**：SQLite users 表、Argon2id 哈希、session token（DB 中存 SHA-256 哈希）。不用 PAM/系统用户。
- **首启引导**：无用户时 `/api/users/bootstrap` 创建首个管理员（无需认证，仅一次）。
- **Jail 走 libjail FFI**（不用 iocage/ezjail 等）——项目要求。
- **Bhyve 走 vm-bhyve**——项目要求。

## 待办

框架（认证、布局、仪表盘、用户管理、审计）+ 监控采集（CPU/内存/负载/温度）已完成。未实现模块返回 "planned" 占位。阶段计划见 `docs/plan/80-roadmap.md`。
