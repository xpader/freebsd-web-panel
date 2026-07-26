# 01 — 用户认证

## 概述

面板自有用户体系（非系统用户/PAM）。Argon2id 密码哈希，随机 session token（SHA-256 哈希存储），中间件强制认证。首启时通过 bootstrap 接口创建首个管理员。双层登录防暴力破解（用户名锁定 + IP 封禁）。

## 实现细节

### 密码哈希 `src/auth.rs`

- `hash_password(plain) -> ApiResult<String>`：Argon2id，`SaltString::generate(&mut OsRng)` 随机盐
- `verify_password(plain, phc) -> ApiResult<()>`：解析 PHC 字符串后 `Argon2.verify_password`，失败返回 `Unauthorized`
- 密码最短 6 位（`handlers/users.rs::validate_password`）

### Session Token

- `mint_token()` 生成：`hex(uuid_v4) + "." + base64url(32字节随机)`，返回 `(明文token, sha256哈希)`
- DB 仅存 `token_hash`（SHA-256），不存明文
- `hash_token(token)` = SHA-256 hex
- 有效期由 `config.auth.session_ttl`（默认 8 小时）控制，存 `expires_at`

### 认证中间件 `src/auth.rs::require_auth`

```
请求 → extract_bearer(Authorization: Bearer <token>)
     → hash_token → db::get_session_by_hash(hash, now)
     → db::get_user(session.user_id)
     → 插入 AuthUser 到 extensions → next.run(req)
```

未通过则返回 `NotAuthenticated`（401），前端 `api.js` 统一处理（见下文"前端 401 流程"）。

### AuthUser 提取器

```rust
impl FromRequestParts<AppState> for AuthUser { ... }
```

从 `request.extensions` 读取中间件注入的 `AuthUser`。Handler 参数列表中声明 `auth: AuthUser` 即可获取当前用户身份。

### 登录防暴力破解 `src/auth.rs::LoginGuard`

双层内存防护，互不依赖：

| 层级 | 触发条件 | 封锁时长 | 配置项 | 默认值 |
|------|---------|---------|--------|--------|
| 用户名锁定 | 同一用户名连续失败 N 次 | `lockout_sec` | `max_login_attempts` / `lockout_sec` | 5 次 / 300s |
| IP 封禁 | 同一 IP 累计失败 M 次（跨所有用户名） | `ip_ban_sec` | `max_ip_login_attempts` / `ip_ban_sec` | 20 次 / 1800s |

**数据结构**：`LoginGuard` 内部维护两个 `HashMap<String, AttemptRecord>`（`by_user` + `by_ip`），每条记录含 `fail_count` + `locked_until`。

**工作流程**（`handlers/auth.rs::login`）：
1. 先查 IP 封禁（`check_ip`）→ 已封禁返回 `IpBanned`（429，error kind = `ip_banned`）
2. 再查用户名锁定（`check_user`）→ 已锁定返回 `AccountLocked`（429，error kind = `account_locked`）
3. 登录失败时同时记录用户名和 IP 的失败计数
4. 登录成功时清除两份记录（`record_success`）

**过期清理**：`check_user` / `check_ip` 在发现条目已过期（`locked_until > 0 && locked_until <= now`）时立即移除，`fail_count` 归零，避免"到期后失败一次就重新锁定"。

**存储位置**：纯内存（`Arc<parking_lot::Mutex<GuardState>>`），进程重启即清空。

**客户端 IP 提取**（`extract_client_ip`）：依次检查 `X-Forwarded-For`（取最左）→ `X-Real-IP` → 连接地址。`main.rs` 使用 `into_make_service_with_connect_info::<SocketAddr>()` 以获取连接地址。

### Bootstrap（首启引导）`handlers/users.rs`

- `GET /api/users/bootstrap`：返回 `{needs_setup, user_count}`，前端据此决定显示登录还是初始化向导
- `POST /api/users/bootstrap`：仅当 `user_count == 0` 时允许，创建首个 admin（无需认证）
- 已有用户后该接口返回 `Conflict`

### 用户管理 CRUD `handlers/users.rs`

| 操作 | 校验 |
|---|---|
| 创建 | 用户名 `^[a-zA-Z0-9_.-]{2,32}$`，密码 ≥ 6 位 |
| 改密 | 密码 ≥ 6 位 |
| 删除 | 禁止删除自己（`id == auth.user_id` → 400） |

### 前端 401 流程 `frontend/src/lib/api.js`

非登录页收到 401 时采用 **forever-pending** 模式（业界主流做法）：

1. `handleSessionExpired()`：登出（清除 token + Pinia 状态）→ `router.replace('/login')` → 弹出"登录已失效"模态框
2. `return new Promise(() => {})`：返回永不 resolve 的 Promise

**效果**：页面的 `catch` 永远不会执行（组件已在跳转中被卸载），各页面**无需任何 401 判断**，不会出现重复弹窗或显示 `unauthenticated` 错误。

**登录页例外**：登录页的 401 = 密码错误，正常 `throw`，LoginPage 显示"用户名或密码错误"。

**并发 401 去重**：模块级 `sessionExpiredHandling` 标志位防止多个并发请求同时触发弹窗。

## API

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| GET | `/api/users/bootstrap` | 否 | 首启状态检查 |
| POST | `/api/users/bootstrap` | 否 | 创建首个管理员 |
| POST | `/api/auth/login` | 否 | 登录，返回 session token（含防暴力破解检查） |
| POST | `/api/auth/logout` | 是 | 登出（删除 session） |
| GET | `/api/auth/me` | 是 | 当前用户信息 |
| GET | `/api/users` | 是 | 用户列表 |
| POST | `/api/users` | 是 | 创建用户 |
| PUT | `/api/users/{id}` | 是 | 修改密码 |
| DELETE | `/api/users/{id}` | 是 | 删除用户 |

**登录错误响应**：

| HTTP 状态 | error kind | 说明 |
|-----------|-----------|------|
| 401 | `unauthorized` | 用户名或密码错误 |
| 429 | `account_locked` | 用户名被锁定（失败次数过多） |
| 429 | `ip_banned` | IP 被封禁（跨用户名累计失败过多） |

## 配置项

```toml
[auth]
session_ttl = 28800            # 会话有效期（秒），默认 8 小时
max_login_attempts = 5         # 用户名锁定阈值
lockout_sec = 300              # 用户名锁定时长（秒）
max_ip_login_attempts = 20     # IP 封禁阈值
ip_ban_sec = 1800              # IP 封禁时长（秒）
```

## 外部依赖

- `argon2` 0.5、`rand` 0.8（OsRng）、`sha2` 0.10、`base64` 0.22、`hex` 0.4

## 已知限制

- 仅支持 `admin` 角色（单管理员模型），RBAC 预留但未实现
- 无密码重置 / 邮箱验证流程
- Session 不支持"记住我"（固定 TTL）
- 防暴力数据纯内存，进程重启即清空
