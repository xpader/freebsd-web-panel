# 30 — 通用后台任务机制

## 概述

`src/bgtask.rs` 提供通用的长运行任务基础设施，支持 stdout/stderr 流式输出到前端。pkg 安装/删除、pkg 仓库更新、vm-bhyve 初始化等长任务共用同一套机制。

## 实现细节

### 数据结构

```rust
pub enum TaskStatus { Running, Done, Failed }

pub struct BgTask {
    id: String,          // 唯一 ID（时间戳 + PID + 纳秒熵）
    kind: String,        // 任务类型标识（"pkg-install" | "pkg-delete" | "pkg-update" | "bhyve-init" | ...）
    label: String,       // 前端显示用描述（如 "pkg install vim"）
    status: TaskStatus,
    exit_code: Option<i32>,
    lines: Vec<String>,  // 累积的 stdout/stderr 行
    created_at: i64,     // Unix 时间戳（用于 GC）
}
```

### 存储

全局 `static TASKS: LazyLock<Mutex<HashMap<String, BgTask>>>`。使用 `parking_lot::Mutex`（同步 mutex，不跨 `.await`）。

任务在创建时自动 GC：超过 10 分钟（`TASK_TTL_SECS = 600`）的任务自动清除。

### 公共 API

| 函数 | 说明 |
|------|------|
| `create(kind, label)` | 创建任务，插入存储，返回 id |
| `push_line(id, line)` | 追加一行输出 |
| `set_status(id, status, exit_code)` | 设置最终状态 |
| `get(id)` | 获取任务快照（clone） |
| `gc()` | 清除过期任务 |
| `run_streaming_cmd(id, cmd, args)` | spawn 命令，逐行捕获 stdout/stderr 推入存储，返回 exit code |
| `stream_handler(...)` | 统一 SSE handler（axum handler） |

### `run_streaming_cmd` 流程

1. `tokio::process::Command` spawn 命令，stdout/stderr 设为 `Stdio::piped()`
2. 分别为 stdout 和 stderr 起 tokio task，用 `BufReader::lines()` 逐行读取
3. 每行通过 `push_line()` 推入任务存储
4. 两个 reader task 完成后 `child.wait()` 获取 exit code
5. 返回 exit code（spawn 失败返回 -1）

### SSE Handler

`GET /api/tasks/{id}/stream`（公开路由，token 经 `?token=` query 参数验证）。

使用 `futures_util::stream::unfold` 构建 SSE 流：
- 每 500ms 读取任务快照，推送 `last_len..` 范围的增量行
- 任务完成（status != Running）后发送最终事件
- 追加 `event("done")` 终止信号，EventSource 自动关闭

SSE 数据 JSON：
```json
{ "status": "running|done|failed", "lines": ["..."], "exit_code": null|int, "kind": "...", "label": "..." }
```

## API

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/api/tasks/{id}/stream` | 公开（token via query） | 任意后台任务的 SSE 输出流 |
| GET | `/api/tasks/{id}` | 需要 | 获取任务状态快照（SSE 错误时 fallback） |
| GET | `/api/pkg/tasks/{id}` | 需要 | 同上（向后兼容保留） |

## 使用方

| 模块 | kind | 触发 API |
|------|------|----------|
| pkg 安装 | `pkg-install` | `POST /api/pkg/install` |
| pkg 删除 | `pkg-delete` | `POST /api/pkg/delete` |
| pkg 仓库更新 | `pkg-update` | `POST /api/pkg/repos/update` |
| bhyve 初始化 | `bhyve-init` | `POST /api/bhyve/init` |

### 各模块使用模式

```rust
// 创建任务
let id = bgtask::create("my-task", "description");

// 后台执行
tokio::spawn(async move {
    // 流式命令
    let exit = bgtask::run_streaming_cmd(&id, "/path/to/cmd", &["arg"]).await;
    // 或手动推入输出
    bgtask::push_line(&id, "some message");
    // 设置最终状态
    bgtask::set_status(&id, bgtask::TaskStatus::Done, Some(exit));
    // 审计日志
    audit::record(...);
});

// 返回 task_id 给前端
Ok(Json(json!({ "task_id": id })))
```

### 前端使用模式

```javascript
const res = await api.post('/api/bhyve/init', { spec: '...' });
const taskId = res.task_id;

const token = sessionStorage.getItem('fwp_token');
const url = `/api/tasks/${taskId}/stream?token=${encodeURIComponent(token)}`;
const es = new EventSource(url);

es.onmessage = (ev) => {
    const data = JSON.parse(ev.data);
    if (data.lines?.length) output += data.lines.join('\n') + '\n';
    if (data.status !== 'running') { /* done/failed */ es.close(); }
};
es.addEventListener('done', () => es.close());
```

## 外部依赖

- Rust crate：`tokio::process`（异步子进程）、`parking_lot`（任务存储 mutex）、`futures_util`（SSE stream 构造）、`axum::response::sse`（Sse + Event + KeepAlive）

## 设计决策

- **统一存储 vs 分模块存储**：选择统一存储（一个 `TASKS` HashMap），因为不同模块的长任务不会同时产生大量任务，且统一 SSE handler 避免重复代码。
- **parking_lot::Mutex vs tokio::sync::Mutex**：任务存储的 lock 不跨 `.await`（仅读/写 HashMap 后立即释放），用同步 mutex 更高效。
- **SSE vs WebSocket**：选择 SSE，因为任务是单向输出（服务器→客户端），不需要双向交互。EventSource 自动重连，且比 WebSocket 更轻量。
