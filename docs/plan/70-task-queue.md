# 设计：任务队列与长时操作

## 1. 需求

部分操作耗时长（ISO 下载、ZFS send/receive、scrub、VM 安装），HTTP 同步请求不适用。
需要：
- 异步执行，立即返回任务 ID
- 查询进度
- 取消
- 流式输出（stdout/stderr 实时推送）

## 2. 模型

```rust
struct Task {
    id: Uuid,
    kind: TaskKind,             // IsoDownload | ZfsSend | ZfsReceive | Scrub | VmInstall | Custom
    state: TaskState,           // Pending | Running | Done | Failed | Cancelled
    progress: Option<f32>,      // 0.0..1.0（若可计算）
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    output: Vec<LogLine>,       // 截断保留最近 N 行
    result: Option<TaskResult>, // 成功返回值/错误
    cancel: CancellationToken,  // tokio util
}

enum TaskKind { IsoDownload, ZfsSend, ZfsReceive, Scrub, VmInstall, Service(String) }
```

## 3. 实现

- 基于 `tokio::task::JoinHandle` + `tokio_util::sync::CancellationToken`
- 任务存储：进程内 `DashMap<Uuid, Task>`（单机面板无需持久化任务状态；重启即清空）
- 进程输出捕获：`tokio::process::Command` + 异步读取 stdout/stderr pipe，逐行写入 Task.output
- 进度计算：
  - ISO 下载：解析 `Content-Length` + 已接收字节
  - scrub：`zpool status` 的 scrub 百分比行
  - 其他：无明确进度时显示"运行中"+ 实时日志

## 4. API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/tasks` | 列出任务（支持 `?state=running`） |
| GET | `/api/tasks/:id` | 任务详情（含输出） |
| GET | `/api/tasks/:id/stream` | WebSocket 实时输出流 |
| POST | `/api/tasks/:id/cancel` | 取消任务（发送 CancellationToken） |

## 5. 前端
- 全局任务指示器（顶栏角标，显示运行中任务数）
- 任务页：列表 + 实时日志终端
- 触发长操作后跳转或显示任务卡片，WebSocket 订阅输出

## 6. 实现里程碑

随各模块需要逐步引入：
- **M-zfs-send** — 首个任务化操作（ZFS send/receive）
- **M-iso** — ISO 下载任务
- **M-scrub** — scrub 进度
- 统一在 `src/task/` 模块，被各业务模块复用
