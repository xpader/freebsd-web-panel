# 34 - 系统电源管理（关机/重启）

## 概述

为 Web 面板提供系统级关机和重启功能。用户在右上角设置菜单中操作，需二次确认。所有操作记录审计日志。

## 实现细节

### 后端

- **源码位置**：`src/handlers/system.rs`（末尾 `shutdown` / `reboot` 函数）
- **命令**：`/sbin/shutdown -p now`（关机）、`/sbin/shutdown -r now`（重启）
- **执行方式**：`tokio::task::spawn_blocking` + `cmd::run_sync`，避免阻塞异步运行时
- **审计**：通过 `audit::record()` 记录操作者和操作类型
- **响应**：立即返回 `{ "status": "shutting_down" }` 或 `{ "status": "rebooting" }`，实际命令在后台线程执行。由于系统即将关机/重启，HTTP 响应可能不会到达客户端。

### 前端

- **位置**：`frontend/src/components/layout/TopBar.vue` 设置下拉菜单底部
- **UI**：分隔线后放置"关机"（`fa-power-off`）和"重启"（`fa-rotate-right`）按钮
- **确认**：通过 `useConfirm()` 弹出确认对话框，防止误操作
- **错误处理**：请求失败时通过 `useAlert()` 弹窗提示

### 样式

- `.dropdown-divider`：1px 分隔线，使用 `var(--border)` 颜色，`4px` 上下间距

## API

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| POST | `/api/system/shutdown` | 是 | 关闭系统 |
| POST | `/api/system/reboot` | 是 | 重启系统 |

### 响应

```json
{ "status": "shutting_down" }
{ "status": "rebooting" }
```

## i18n

| Key | en | zh |
|---|---|---|
| `topbar.shutdown` | Shutdown | 关机 |
| `topbar.shutdownConfirm` | Are you sure you want to shut down the system? The panel will become unavailable. | 确定要关闭系统吗？面板将不可用。 |
| `topbar.reboot` | Reboot | 重启 |
| `topbar.rebootConfirm` | Are you sure you want to reboot the system? The panel will be temporarily unavailable. | 确定要重启系统吗？面板将暂时不可用。 |

## 已知限制

- 关机/重启后，WebSocket 和 SSE 连接会断开，前端不会收到明确的成功反馈。确认对话框已说明"面板将不可用"。
- 无延迟关机选项（当前为 `now`，立即执行）。
