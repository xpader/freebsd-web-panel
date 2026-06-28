# 07 — 静态资源服务

## 概述

前端资源（`web/` 目录）通过两种方式服务：磁盘优先（开发模式，改文件即时生效）+ 编译期内嵌（`rust-embed`，生产单二进制）。`embed-web` 默认开启。

## 实现细节

### 编译期嵌入 `src/web_assets.rs`

```rust
#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAsset;
```

`rust-embed` 在编译时把 `web/` 目录所有文件嵌入二进制。`WebAsset::get(path)` 返回 `Option<EmbeddedFile>`。

### Handler `serve_asset`

作为 Router 的 `fallback` handler 处理所有非 `/api/` 路径：

```
请求 → State<AppState> + Request
  → path = req.uri().path()
  → 磁盘优先：if web_root 存在且文件存在 → 读文件返回
  → 内嵌回退：WebAsset::get(path) → 返回内嵌字节
  → SPA 回退：无扩展名路径 → 返回内嵌 index.html（hash 路由）
  → 404
```

### MIME 类型推断

`mime_guess_for(path)` 按扩展名硬编码映射（html/css/js/json/svg/png/jpg/gif/ico/woff2/woff），未知返回 `application/octet-stream`。

### 开发模式 vs 生产模式

| 模式 | 行为 |
|---|---|
| **开发** | `web_root = "web"`（仓库目录），磁盘文件优先，改前端无需重新编译 |
| **生产** | `web_root` 不存在或指向不存在的路径，自动回退到内嵌资源，任意工作目录可运行 |

### SPA 路由回退

`has_extension(name)` 判断路径是否含 `.`（如 `/dashboard` 无扩展名 → 是 SPA 路由）。无扩展名的请求返回 `index.html`，前端 hash router 接管。

### Feature Flag

```toml
[features]
default = ["embed-web"]
embed-web = ["dep:rust-embed"]
```

`embed-web` 关闭时 `WebAsset` 不可用，仅依赖磁盘 `web_root`。默认开启。

## 配置项

| 字段 | 默认值 | 说明 |
|---|---|---|
| `server.web_root` | `/usr/local/share/fwp/web` | 磁盘资源目录（开发时设为 `web`） |

## 外部依赖

- `rust-embed` 8（编译期嵌入，默认开启）

## 已知限制

- 内嵌资源后修改前端必须重新 `cargo build`
- 无 ETag / Cache-Control 头（浏览器每次完整下载）
- 无 gzip 压缩
