# 25 — 内存占用与性能优化

跨模块的性能与内存优化笔记。各条目的具体实现细节仍由各自功能文档详述，本文只汇总**模式、动因、收益**，便于新开发者快速了解"为什么这么写"。

## 背景

`fwp` 以 root 长期驻留，处理来自浏览器的系统管理请求。在 FreeBSD 上使用 jemalloc（默认分配器），观察到以下现象：

- `top` 中 SIZE/RES 在频繁 ZFS 操作、文件上传后单调增长，不再回落。
- 并非真正的内存泄漏（`jemalloc` 没有未释放的块），而是 `free()` 不归还物理页给操作系统——jemalloc 保留在 thread/cache arena 中复用。
- 因此**减少峰值分配**比"事后归还"更有效：一次分配都不发生，jemalloc 自然没有页可保留。

优化方向（按优先级）：

1. **减少峰值分配**（本文重点）——流式 I/O、缓存编译产物、消除双分配。
2. 调整 jemalloc 配置（`MALLOC_CONF` 的 `dirty_decay_ms` / `muzzy_decay_ms`）——运行期调优，未实施。
3. 切换到 `mimalloc` 或 `snmalloc`——跨平台差异大，未实施。

## 优化项一览

| # | 模块 | 原实现 | 优化后 | 收益 |
|---|---|---|---|---|
| 1 | 文件上传 `handlers/files.rs` | `body: Bytes` 聚合整个 body 再 `std::fs::write` | `body: Body` + `into_data_stream()` 分块写盘 + `.upload-tmp.<filename>` + 原子 rename | 峰值内存从 ~2× 文件大小降到 ~64 KB；中断/覆盖场景更安全 |
| 2 | 文件下载 `handlers/files.rs` | `std::fs::read` 整文件 → `Body` | `tokio::fs::File` + `tokio_util::io::ReaderStream` + `Body::from_stream` | 峰值内存从 ~文件大小降到 ~8 KB |
| 3 | ZFS 输入校验 `handlers/zfs.rs` | 每个请求 N 次 `Regex::new()`（N = 数据集数） | 6 个 `static LazyLock<Regex>` 进程生命周期只编译一次 | 消除每请求 ~数十 µs 编译开销 + 大量临时堆分配 |
| 4 | 命令 stdout 提取 `cmd.rs` | `String::from_utf8_lossy(&v).to_string()`（两次分配） | `run_sync_str` 用 `String::from_utf8(v).map_err(lossy)`；`output_ok` 用 `lossy.into_owned()` | 合法 UTF-8 路径零/一次分配（原为两次） |

## 模式与动因

### 模式 A：流式 I/O

**反例**：
```rust
pub async fn upload(body: Bytes) -> ... {
    std::fs::write(&dest, &body)?;     // body 在聚合阶段已占满文件大小
}
```
axum 的 `Bytes` 提取器会把整个 body 先攒到内存，handler 才拿到引用。上传 500 MB 时：
- 聚合阶段分配 ~500 MB（增长中还会按 2 倍扩，峰值 ~1 GB）。
- `std::fs::write` 再写盘，内核页缓存又占一份。

**修正**：
```rust
pub async fn upload(body: axum::body::Body) -> ... {
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(&tmp, &dest).await?;
}
```
chunk 到达就立刻写盘、drop，常驻只有 ~64 KB（hyper 默认 chunk 大小）。

同样的思路用于下载：`tokio::fs::File` + `ReaderStream` 把文件分块喂给 `Body::from_stream`，避免 `std::fs::read` 的整文件分配。

### 模式 B：静态缓存编译产物

**反例**：
```rust
fn validate_name(name: &str) -> ApiResult<()> {
    let re = Regex::new(r"^[a-zA-Z0-9@/_:\-\.]+$").unwrap();  // 每请求都编译
    if !re.is_match(name) { ... }
}
```
`Regex::new` 内部编译 DFA，单次 ~数十 µs 加一次堆分配。列表页一次请求触发 N 次（N = 数据集数）。

**修正**：
```rust
static RE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"...").unwrap());
fn validate_name(name: &str) -> ApiResult<()> {
    if !RE_NAME.is_match(name) { ... }
}
```
`std::sync::LazyLock`（Rust 1.80+）保证一次编译、线程安全、零运行期开销。

**适用边界**：
- 模式是编译期已知的常量正则 → `LazyLock<Regex>`。
- 模式依赖运行期输入 → 仍然用 `Regex::new`（必要时包一层 `HashMap` 缓存）。

### 模式 C：消费式 UTF-8 转换避免双分配

`Command::output().stdout` 是 `Vec<u8>`，要转 `String`。

**反例**：
```rust
String::from_utf8_lossy(&v).to_string()
```
- `from_utf8_lossy(&v)`：合法 UTF-8 返回 `Cow::Borrowed(&str)`，非法返回 `Cow::Owned(String)`（已分配一次）。
- `.to_string()`：无论 `Cow` 是 Borrowed 还是 Owned，都再 clone 一次。

**修正 1（消费式，零拷贝）**：
```rust
String::from_utf8(v).map_err(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
```
`from_utf8(v)` 消费 `Vec<u8>`，合法 UTF-8 直接复用原 buffer 返回 `String`，**零额外分配**。

**修正 2（借用式，单次 clone）**：
```rust
String::from_utf8_lossy(&v).into_owned()
```
`into_owned()` 对 Borrowed 变体 clone 一次，对 Owned 变体直接返回。只在非法 UTF-8 路径上分配一次。

## 验证方法

**FWP 状态页面**：监控菜单下的「FWP 状态」页面（`MonitorFwpPage.vue`）调用 `/api/debug/jemalloc-stats` 接口，实时展示内存分类。

**观察 RES 趋势**：
```sh
while :; do
  ps -o pid,rss,vsz,command -p $(pgrep fwp) | tail -1
  sleep 5
done
```
优化前：每做一次 ZFS 列表/文件上传，RSS 阶梯式上升且不回落。优化后：RSS 在操作期间短暂上升，操作结束后回落（jemalloc 把不活跃的页归还给 OS 需要几秒到几十秒）。

**上传峰值内存**：上传 500 MB 文件时观察 RSS，优化前峰值 ~1 GB，优化后 ~30 MB（包含 tokio worker 基线）。

**单元测试**：`cargo test --offline --bin fwp` 27 个测试全过。

### jemalloc 驻留 vs 进程 RSS

`jemalloc` 的 `stats.resident` **只统计 jemalloc 自身 mmap 的物理页**。进程总 RSS 还包括大量非 jemalloc 管理的内存：

- 代码段（.text）：Rust debug build 含完整调试符号，可达数十 MB
- 线程栈：每个 tokio worker 线程 ~2 MB
- 共享库映射：libc、libssl 等
- 直接 mmap 区域：Rust std 或第三方 crate 绕过 jemalloc 的分配

实测 debug build 中，jemalloc `resident` 通常仅占进程 RSS 的 30% 左右。

`/api/debug/jemalloc-stats` 额外返回 `process_rss` 字段——通过 `sysctl(KERN_PROC_PID)` 读取 `kinfo_proc.ki_rssize`（FreeBSD 15 amd64 上为 `segsz_t` = int64，位于 struct 偏移 264 处，以页为单位）。前端据此将进程 RSS 分为四类：

| 分类 | 计算方式 | 含义 |
|---|---|---|
| 程序在用 | `allocated` | jemalloc 中活的 Rust 对象 |
| 进程其它内存 (Other Memory) | `process_rss − resident` | 代码段、调试符号、栈、共享库 |
| 可被系统回收 | `resident − active` | jemalloc 脏页 / 模糊页，OS 可回收 |
| 不可回收开销 | `active − allocated` | jemalloc 内部碎片 |

## 尚未实施的方向

以下方向经评估但暂未实施，留作后续优化参考：

1. **jemalloc 调优**：设置 `MALLOC_CONF=dirty_decay_ms:1000,muzzy_decay_ms:5000` 可加速页归还；需实测确认对吞吐的影响。
2. **替换分配器**：`mimalloc` / `snmalloc` 在多线程高分配场景下通常比 jemalloc 更友好，但需跨平台验证（项目目标平台 FreeBSD amd64）。
3. **启动清理**：`handlers/files.rs` 的 `.upload-tmp.*` 残留文件未在启动时清理（仅在正常失败路径删除），可加一次 glob 扫描。
4. **`tokio::io::copy`**：上传循环当前用 `file.write_all(&chunk)`，等价于 `tokio::io::copy(&mut stream, &mut file)`；后者略简洁，性能相同。
5. **大命令输出的流式解析**：`zfs list` / `pkg query` 输出巨大时整个 stdout 在 `Output` 中占用。可改为 `spawn().stdout = Stdio::piped()` + `BufReader` 逐行解析，避免峰值分配。影响面较大，留作下一步。

## 涉及源码

| 文件 | 优化项 |
|---|---|
| `src/handlers/files.rs` | #1 上传流式 + 原子提交、#2 下载流式 |
| `src/handlers/zfs.rs` | #3 静态 LazyLock 正则缓存 |
| `src/handlers/debug.rs` | jemalloc-stats + process_rss（sysctl KERN_PROC_PID） |
| `src/cmd.rs` | #4 UTF-8 双分配修复 |
| `Cargo.toml` | 新增 `tokio-util = "0.7"`（io feature，用于 `ReaderStream`） |
