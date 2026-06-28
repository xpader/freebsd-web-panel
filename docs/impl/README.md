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

## 文档规范

每篇实现文档应包含：

1. **概述** — 功能目标与边界
2. **实现细节** — 关键数据结构、算法、调用链、源码位置（精确到文件:行范围）
3. **API** — 接口列表（方法/路径/请求/响应）
4. **外部依赖** — 系统命令、crate、第三方库
5. **配置项** — 相关 `fwp.toml` 字段
6. **已知限制 / TODO** — 当前未覆盖的部分
