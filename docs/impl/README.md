# 实现文档索引

> 本目录记录**已实现**功能的实际实现逻辑（区别于 `docs/plan/` 中的设计计划）。

## 文档列表

| 文档 | 功能 | 涉及源码 |
|---|---|---|
| [00-framework.md](00-framework.md) | 项目骨架：入口、路由、配置、错误处理、状态管理 | `main.rs`, `app.rs`, `state.rs`, `config.rs`, `error.rs` |
| [01-auth.md](01-auth.md) | 用户认证：密码哈希、会话管理、中间件、首启引导 | `auth.rs`, `handlers/auth.rs`, `handlers/users.rs`, `db.rs` |
| [02-database.md](02-database.md) | SQLite 数据库：连接管理、表结构、数据访问层 | `db.rs` |
| [03-audit.md](03-audit.md) | 审计日志：追加式记录、查询 | `audit.rs`, `handlers/audit.rs` |
| [04-system-metrics.md](04-system-metrics.md) | 系统指标：CPU/内存/Swap/温度/负载实时采样 | `handlers/system.rs` |
| [05-monitoring.md](05-monitoring.md) | 监控采集：后台任务、时序存储、查询 API、图表前端 | `monitor.rs`, `db.rs`, `web/js/pages/monitor.js` |
| [06-frontend.md](06-frontend.md) | 前端架构：SPA 路由、API 封装、两级菜单、布局 | `web/js/*.js`, `web/css/app.css` |
| [07-web-assets.md](07-web-assets.md) | 静态资源服务：rust-embed 内嵌 + 磁盘回退 | `web_assets.rs` |
| [08-filesystem.md](08-filesystem.md) | 文件系统：概览（磁盘/挂载点/池）+ 磁盘详情（分区表） | `handlers/filesystem.rs`, `web/js/pages/filesystem.js`, `web/js/pages/disks.js` |
| [09-zfs.md](09-zfs.md) | ZFS 管理：Zpool/数据集/快照 + 三级菜单 | `handlers/zfs.rs`, `web/js/pages/zfs.js` |
| [10-file-manager.md](10-file-manager.md) | 文件管理器：目录树 + 列表/网格、上传/下载/重命名/删除/属性 | `handlers/files.rs`, `web/js/pages/files.js` |
| [11-accounts.md](11-accounts.md) | 系统用户与用户组：解析 /etc/passwd、/etc/group 的只读列表 | `handlers/accounts.rs`, `web/js/pages/accounts.js` |
| [12-i18n.md](12-i18n.md) | 国际化：i18next 多语言（中文/英文）、顶栏国旗切换 | `web/js/i18n/`, `web/js/ui/layout.js`, `web/vendor/i18next.min.js` |
| [13-sysinfo.md](13-sysinfo.md) | sysctl(3) 共享读取器：CPU/内存/温度/负载（替代子进程） | `src/sysinfo.rs` |
| [14-terminal.md](14-terminal.md) | Web 终端：WebSocket ↔ FreeBSD PTY（xterm.js 前端，root 登录 shell） | `src/terminal.rs`, `web/js/pages/terminal.js`, `web/vendor/xterm/` |
| [15-rcconf.md](15-rcconf.md) | RC 配置：列出/新增/修改/删除 rc.conf 变量（sysrc 异步 API） | `handlers/rcconf.rs`, `frontend/src/pages/RcconfPage.vue` |
| [16-crontab.md](16-crontab.md) | 定时任务：列出/新增/修改/删除/启停 crontab 条目（crontab） | `handlers/crontab.rs`, `web/js/pages/cron.js` |
| [17-network.md](17-network.md) | 网络接口管理：接口列表/路由表/默认网关（IPv4+IPv6 读写）、rc.conf 配置（DHCP/SLAAC/Static）、虚拟接口创建/销毁 | `handlers/network.rs`, `frontend/src/pages/NetworkPage.vue` |
| [18-services.md](18-services.md) | 服务管理：列出 rc.d 服务（启用/运行状态）、start/stop/restart 控制 | `handlers/services.rs`, `web/js/pages/services.js` |
| [19-sysctl.md](19-sysctl.md) | sysctl 浏览：列出全部内核参数（值/类型/描述/修改状态），搜索+分页 | `handlers/sysctl.rs`, `web/js/pages/sysctl.js` |
| [20-jail.md](20-jail.md) | Jail 容器：libjail FFI 运行时查询、基础系统管理（导入/镜像创建：ZFS Clone/unionfs/sharedfs） | `jail.rs`, `handlers/jails.rs`, `web/js/pages/jails.js` |
| [21-pkg.md](21-pkg.md) | pkg 软件包管理：列出已安装包（全部/手动/自动）、查看包详情（描述/依赖/反向依赖/文件列表） | `handlers/pkg.rs`, `web/js/pages/pkg.js` |
| [22-pkg-repos.md](22-pkg-repos.md) | pkg 软件源配置管理：仓库 CRUD（UCL 解析/生成）、启用/禁用、`pkg update -f` 后台任务、预设镜像模板 | `handlers/pkg.rs`, `web/js/pages/pkg-repos.js` |
| [23-bhyve.md](23-bhyve.md) | Bhyve 虚拟机管理：VM 列表/详情/创建/启停、串口控制台（cu + nmdm）、VNC 代理（WS→TCP）、镜像/交换机/数据存储/模板/ISO 列表 | `bhyve.rs`, `handlers/bhyve.rs`, `terminal.rs`, `frontend/src/pages/Bhyve*.vue` |
| [24-cmd.md](24-cmd.md) | 命令执行封装：spawn_blocking 统一封装、async/sync 函数选择、管道 FD 死锁问题 | `cmd.rs` |
| [25-memory.md](25-memory.md) | 内存占用与性能优化：流式 I/O、静态正则缓存、UTF-8 双分配修复、jemalloc 行为分析 | `handlers/files.rs`, `handlers/zfs.rs`, `cmd.rs` |
| [26-sysrc.md](26-sysrc.md) | sysrc 统一封装：rc.conf 读写的唯一入口，同步+异步 API、export 解析器 | `sysrc.rs` |

## 文档规范

每篇实现文档应包含：

1. **概述** — 功能目标与边界
2. **实现细节** — 关键数据结构、算法、调用链、源码位置（精确到文件:行范围）
3. **API** — 接口列表（方法/路径/请求/响应）
4. **外部依赖** — 系统命令、crate、第三方库
5. **配置项** — 相关 `fwp.toml` 字段
6. **已知限制 / TODO** — 当前未覆盖的部分
