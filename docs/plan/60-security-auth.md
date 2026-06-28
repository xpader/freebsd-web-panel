# 设计：安全与认证

## 1. 威胁模型

面板以 root 运行，能执行任意系统命令，因此 Web 接口本身是主要攻击面。
核心目标：**防止未授权访问**，其次是审计与最小权限日志。

## 2. 监听与网络暴露

- **默认仅监听 `127.0.0.1`**（本地回环），端口 `8443`
- 远程访问两种受控方式：
  - 配置文件显式指定监听地址（如内网 IP）
  - 前置反向代理（nginx/caddy）+ 网络层 ACL
- **默认启用 TLS**：首次启动自动生成自签名证书存于 `/var/db/fwp/cert.pem`、`/var/db/fwp/key.pem`；可配置替换为正式证书
- HTTP 明文模式仅在配置 `tls.enable = false` 时启用（仅供反代场景，并打告警日志）

## 3. 认证

### 3.1 Token 认证（默认，开箱即用）

- **首启生成初始 token**：32 字节随机（`/dev/urandom`），hex 编码，写入 `/var/db/fwp/token`（权限 `0600`），仅在终端日志打印一次
- 客户端通过 `POST /api/auth/login` 提交 token（或 PAM 凭据换取 session token）
- 成功后签发 **session token**（HMAC-SHA256，payload 含用户标识 + 过期时间），存于 `sessionStorage`
- 所有 API 请求头携带 `Authorization: Bearer <session-token>`
- session 默认 8 小时过期，可配置；支持登出撤销

### 3.2 PAM 认证（可选）

- 配置 `[auth] mode = "pam"`
- `POST /api/auth/login` 接收 `username` + `password`，调用 FreeBSD `pam_unix`（通过 `pam` crate 或直接 FFI `pam_start/pam_authenticate`）
- 仅允许配置的 `allowed_users`（默认 `root` 及 wheel 组）登录
- 成功后同样签发 session token

### 3.3 双因素（后期可选）
TOTP 支持，配置文件绑定 secret。初版不实现。

## 4. 授权

- **初版单管理员模型**：认证通过即全权限
- 预留 RBAC 接口（角色：admin/operator/readonly），但初版仅 admin
- 敏感操作（destroy/rollback/pf 编辑）即便 admin 也强制二次确认（前端 + 审计）

## 5. 请求安全

- 所有 `/api/*` 强制认证（除 `/api/auth/login`）
- CSRF 防护：仅接受带有效 Bearer token 的请求（非 cookie，天然防 CSRF）；若改用 cookie 认证则加 CSRF token
- 输入校验：所有命令参数经白名单/正则校验后再拼入子进程，**禁止 shell 字符串拼接**
  - 例：jailname 仅允许 `[a-zA-Z0-9_-]`；IP 地址用类型化解析
- 子进程统一用 `Command::arg()`（不经过 shell），杜绝命令注入
- 速率限制：`/api/auth/login` 失败 5 次/分钟内封禁该来源 IP 一段时间

## 6. 审计日志

- 全部**写操作**（POST/PUT/DELETE）记录：时间、用户、来源 IP、方法、路径、请求体摘要、结果（成功/失败）、错误信息
- 日志格式：结构化 JSON 行，写入 `/var/db/fwp/audit.log`（轮转）
- 只读 GET 不记录（避免噪音），但危险查询（如导出配置）可标记记录
- 前端提供审计日志查看页（仅 admin）

## 7. 证书与密钥管理

- 自签名证书首次自动生成（CN=hostname，SAN 含所有监听地址）
- token 文件 `/var/db/fwp/token` 权限 `0600`，属主 root
- HMAC 签名密钥派生自 token（或独立 `/var/db/fwp/hmac.key`）
- 配置文件 `/usr/local/etc/fwp.toml` 权限 `0640`（可能含敏感配置）

## 8. 配置示例

```toml
[server]
listen = "127.0.0.1:8443"     # 远程访问改为 "0.0.0.0:8443" 或内网 IP

[tls]
enable = true                  # 默认 true
cert = "/var/db/fwp/cert.pem"
key  = "/var/db/fwp/key.pem"

[auth]
mode = "token"                 # "token" | "pam"
session_ttl_sec = 28800        # 8h
# allowed_users = ["root"]     # pam 模式生效
# allowed_groups = ["wheel"]

[audit]
log = "/var/db/fwp/audit.log"
max_size_mb = 50
keep = 5
```

## 9. 实现里程碑

1. **M1 — 自签名证书自动生成 + TLS 监听**
2. **M2 — Token 认证（初始 token + session token + 中间件）**
3. **M3 — 审计日志中间件**
4. **M4 — PAM 认证（可选）**
5. **M5 — 登录速率限制**
