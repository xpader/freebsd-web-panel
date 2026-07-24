# 29 — 系统邮件 (mbox)

## 概述

管理 FreeBSD 系统 `/var/mail/$USER` 中的 mbox 格式邮件。对应系统 `mail(1)` 命令的常见操作：浏览邮件列表、阅读邮件、删除邮件、标记已读/未读。典型用途是查看 cron 任务输出、安全通知等系统邮件，避免 SSH 登录后用 `mail` 命令操作。

## 实现细节

### mbox 格式

BSD mbox 将所有邮件存储在单一文件中，每封邮件由 **envelope 行**（`From sender Day Mon D HH:MM:SS YYYY`）起始，后接 RFC 2822 头部、空行、正文。`Status:` 头记录已读状态：含 `R` = 已读，含 `O` = 已读但旧，不含 `R` = 未读。

### 解析流程

1. **`split_mbox(content)`** — 逐行扫描，用正则 `^From \S+ \w{3} \w{3} [ \d]?\d \d{2}:\d{2}:\d{2} \d{4}` 匹配 envelope 行作为分隔。每条消息保留原始文本（含换行符）。
2. **`parse_headers(msg)`** — 跳过 envelope 行，解析至首个空行。支持折叠续行（以空格/Tab 开头的行）。
3. **`get_body(msg)`** — 跳过 envelope 行 + 头部，返回正文。

### 写入操作

- **删除**：解析全部消息 → 过滤掉目标索引 → 拼接写回。
- **标记已读**：修改 `Status:` 头（添加/移除 `R`）→ 仅重写该条消息 → 拼接写回。
- **清空邮箱**：`fs::write(path, "")` 截断文件。
- **批量删除**：接受索引集合，一次写入过滤后的结果。

`set_message_read()` 以行为单位操作，保持原有格式不丢失头部/正文内容。若无 `Status:` 头且需要标为已读，在头部末尾插入 `Status: R`。

### 用户名校验

`valid_user()` 确保用户名仅含 `[a-zA-Z0-9_.-]`，防止路径遍历攻击。

### 涉及源码

| 文件 | 说明 |
|---|---|
| `src/handlers/mail.rs` | mbox 解析、所有 handler |
| `src/app.rs` | 路由注册 |
| `frontend/src/pages/MailPage.vue` | 邮件列表 + 详情 overlay |

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/mail/boxes` | 列出所有非空邮箱（用户名、大小、邮件数、未读数、修改时间） |
| GET | `/api/mail/{user}` | 列出指定邮箱的所有邮件摘要（From/To/Subject/Date/已读/大小） |
| GET | `/api/mail/{user}/{index}` | 读取单封邮件（完整头部 + 正文），自动标为已读 |
| DELETE | `/api/mail/{user}/{index}` | 删除单封邮件 |
| POST | `/api/mail/{user}/delete` | 批量删除，body: `{"indices": [0, 1, 2]}` |
| DELETE | `/api/mail/{user}` | 清空整个邮箱 |
| PUT | `/api/mail/{user}/{index}/read` | 标为已读 |
| PUT | `/api/mail/{user}/{index}/unread` | 标为未读 |

## 外部依赖

无外部命令依赖。纯 Rust 文件读写 + 正则匹配。

## 配置项

无。邮箱路径硬编码为 `/var/mail/`（FreeBSD 标准）。

## 已知限制 / TODO

- 大邮箱（如 root 40MB / 938 封）：列表用服务端分页（每页 50 封），只解析当前页的邮件头部，其余仅统计偏移和已读状态。
- `list_mailboxes` 用轻量 `count_messages` 扫描——只数 envelope 行和检查 `Status:` 头，不解析 From/To/Subject/Date。
- 不支持回复/转发/发送邮件（系统通知查看场景，不需要 MUA 功能）。
- 不支持 Maildir 格式（FreeBSD 默认 mbox）。
- 邮件索引为文件内位置序号（0-based），删除操作后索引重排。
- 不支持 `.forward` 文件管理（未来可扩展）。
