# 01 — 用户认证

## 概述

面板自有用户体系（非系统用户/PAM）。Argon2id 密码哈希，随机 session token（SHA-256 哈希存储），中间件强制认证。首启时通过 bootstrap 接口创建首个管理员。

## 实现细节

### 密码哈希 `src/auth.rs`

- `hash_password(plain) -> ApiResult<String>`：Argon2id，`SaltString::generate(&mut OsRng)` 随机盐
- `verify_password(plain, phc) -> ApiResult<()>`：解析 PHC 字符串后 `Argon2::verify_password`，失败返回 `Unauthorized`
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

未通过则返回 `NotAuthenticated`（401），前端 router 收到 401 自动跳转 `#/login`。

### AuthUser 提取器

```rust
impl FromRequestParts<AppState> for AuthUser { ... }
```

从 `request.extensions` 读取中间件注入的 `AuthUser`。Handler 参数列表中声明 `auth: AuthUser` 即可获取当前用户身份。

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

## API

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| GET | `/api/users/bootstrap` | 否 | 首启状态检查 |
| POST | `/api/users/bootstrap` | 否 | 创建首个管理员 |
| POST | `/api/auth/login` | 否 | 登录，返回 session token |
| POST | `/api/auth/logout` | 是 | 登出（删除 session） |
| GET | `/api/auth/me` | 是 | 当前用户信息 |
| GET | `/api/users` | 是 | 用户列表 |
| POST | `/api/users` | 是 | 创建用户 |
| PUT | `/api/users/{id}` | 是 | 修改密码 |
| DELETE | `/api/users/{id}` | 是 | 删除用户 |

## 外部依赖

- `argon2` 0.5、`rand` 0.8（OsRng）、`sha2` 0.10、`base64` 0.22、`hex` 0.4

## 已知限制

- 仅支持 `admin` 角色（单管理员模型），RBAC 预留但未实现
- 无密码重置 / 邮箱验证流程
- Session 不支持"记住我"（固定 TTL）
