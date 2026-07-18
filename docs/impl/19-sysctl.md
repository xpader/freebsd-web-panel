# sysctl 管理：内核参数浏览（只读）

## 概述

列出所有 sysctl 内核参数，展示每项的运行时值、类型、描述文字、可写标志，
以及是否在 `/etc/sysctl.conf` 中被显式修改。当前为只读浏览，后续可扩展写入和持久化。

## 实现细节

### 数据采集 — sysctl(3) C API

**不使用子进程**，直接通过 `libc::sysctl()` 系统调用遍历 MIB 树并获取每个
OID 的元数据和值。全量遍历 ~11k 节点仅需 ~80ms。

FreeBSD sysctl(3) 提供一组「magic」meta-queries（MIB 前两位为 `{0, op}`）：

| 常量 | 值 | 用途 |
|------|---|------|
| `CTL_SYSCTL_NEXTNOSKIP` | 7 | 遍历下一个 OID（包括 CTLFLAG_SKIP 的） |
| `CTL_SYSCTL_NAME` | 1 | 获取 OID 的字符串名称 |
| `CTL_SYSCTL_OIDFMT` | 4 | 获取 kind（类型+标志位）和格式字符串 |
| `CTL_SYSCTL_OIDDESCR` | 5 | 获取描述文字 |

遍历算法：
1. 从空 MIB 开始
2. 调用 `{0, 7, ...current_mib}` 获取下一个 MIB（int 数组）
3. 对每个 MIB 分别调用 NAME、OIDFMT、OIDDESCR 获取元数据
4. 对非 NODE 类型，调用 `sysctl(mib, ...)` 获取值

使用 `NEXTNOSKIP`（7）而非 `NEXT`（2），以包含带 `CTLFLAG_SKIP` 的 compat
别名（如 `kern.ipc.somaxconn`）。

源码：`src/handlers/sysctl.rs`

### kind 字段解析

`OIDFMT` 返回的前 4 字节是 `unsigned int kind`，包含类型和标志位：

- **类型**：`kind & 0xf`（1=node, 2=int, 3=string, 5=opaque, 6=uint, 7=long,
  8=ulong, …）
- **可写**：`kind & 0x40000000`（`CTLFLAG_WR`）

### 值格式化

根据类型将原始二进制 buffer 转为显示字符串：

| 类型 | 格式化方式 |
|------|-----------|
| string (3) | 去 NUL 后直接 UTF-8 |
| int/uint/long 系列 (2,4,6,7,8,9,0xa-0xf) | `to_string()` |
| opaque (5) | `"opaque (N bytes)"` + fmt 字段标注结构体名 |

### 修改检测

解析 `/etc/sysctl.conf`：非注释、非空行中 `=` 左侧的名称视为「已修改」。

### 数据模型

```rust
struct SysctlEntry {
    name: String,
    value: Option<String>,      // None = 读取失败或不可读
    type: String,               // "string" / "integer" / "opaque" / ...
    fmt: String,                // 原始格式字符串（"A", "I", "LU", "S,timeval"…）
    description: Option<String>,
    writable: bool,             // CTLFLAG_WR
    modified: bool,             // 在 /etc/sysctl.conf 中
}
```

### 前端

- 一次性加载全部数据（~11k 条，JSON ~2MB），客户端搜索/过滤/分页
- 搜索框：按名称、值、描述实时过滤
- 「仅已修改」复选框：快速筛选 sysctl.conf 中的条目
- 分页：每页 100 条，支持页码跳转和上一页/下一页
- 修改项：橙色 "Modified" 徽章 + 行高亮
- 可写项：蓝色 "Writable" 徽章
- 可写参数显示「编辑」按钮；已持久化参数显示「重置」按钮
- 编辑流程：formModal 输入新值 → PUT API（运行时生效 + 持久化到 sysctl.conf）
- 重置流程：确认对话框 → DELETE API（从 sysctl.conf 移除）

源码：`web/js/pages/sysctl.js`

## API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/sysctl` | 列出全部 sysctl（按名称排序），可选 `?q=` 过滤 |
| PUT | `/api/sysctl/{name}` | 设置运行时值并持久化到 sysctl.conf（重启后仍生效） |
| DELETE | `/api/sysctl/{name}` | 从 sysctl.conf 移除（恢复默认值，重启后生效） |

### 备份机制

每次写入或删除 sysctl.conf 条目前，先备份当前 `/etc/sysctl.conf` 到
`/var/db/fwp/sysctl-backup/sysctl.conf.<unix-timestamp>`，保留最近 5 份。
与 crontab、resolv.conf 的备份机制一致。

### 运行时写入

通过 `sysctl(3)` 系统调用直接写入（不 spawn `/sbin/sysctl`）。值字符串根据
OID 类型编码为二进制（int → 4 字节 NE、string → NUL 结尾、u64 → 8 字节 NE 等）。
只读参数（无 CTLFLAG_WR）返回 400 错误。

## 外部依赖

- `libc::sysctl()` — 纯系统调用，无子进程
- `/etc/sysctl.conf` — 读取（修改检测）+ 原子写入（持久化）
- `libc::sysctl()` — 读取 + 运行时写入

## 已知限制

- **opaque 类型**：仅显示字节数和格式描述符，不可编辑；运行时写入仅支持数值和字符串类型
  （如 `kern.boottime` 显示为 `opaque (16 bytes)` 而非解析 timeval）
- **无缓存**：每次请求执行全量遍历（~80ms），高频调用时可考虑加 TTL 缓存
