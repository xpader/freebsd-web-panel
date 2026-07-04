# pkg 软件包管理

## 概述

查看系统中通过 `pkg` 安装的所有软件包，包括手动安装包（prime-list）、自动安装包（依赖），以及各个包的详细信息：描述、依赖关系（依赖于什么 / 被什么依赖）、文件列表。当前为只读浏览功能，不包含安装/升级/删除操作。

## 设计决策

### 使用命令行还是系统 API？

**选择：通过 `Command::new("/usr/sbin/pkg")` 调用 `pkg query` 命令。**

理由：
1. **与项目约定一致** — AGENTS.md 明确："FreeBSD 管理通过 spawn 系统二进制并传校验过的参数完成"。services、rcconf、crontab 等模块均遵循此模式。
2. **`pkg query` 格式字符串精确控制输出** — 支持 `%n`（名称）、`%v`（版本）、`%dn`（依赖名）等格式符，输出 TSV，解析简单可靠。
3. **避免 libpkg FFI 的复杂性** — libpkg 不是系统共享库（`/usr/local/lib/libpkg.so` 属于 pkg 自身），引入 FFI 依赖得不偿失。
4. **只读操作安全且快速** — `pkg query` 读取本地 SQLite 数据库（`/var/db/pkg/local.sqlite`），通常在毫秒级完成。

### 模块位置

放在 **Config（配置）** 导航组中，位于 Services 之后。包管理属于系统配置范畴，与服务管理、系统账户同类。

## 实现细节

### 数据结构

```rust
// 列表项摘要
PackageSummary { name, version, origin, comment, automatic, size, homepage, maintainer, install_timestamp }

// 详情（含依赖关系）
PackageDetail { name, version, origin, prefix, comment, description, homepage, maintainer,
                automatic, locked, vital, size_human, size_bytes, arch, abi, repository,
                install_timestamp, categories, licenses, license_logic,
                dependencies, reverse_dependencies }

DepInfo { name, version }              // 依赖项（名称 + 版本）
PackageFile { path, owner, group, mode }  // 文件列表项
```

### `pkg query` 调用链

所有源码位于 `src/handlers/pkg.rs`。

#### 1. 列表 `list_packages`

```
pkg query '%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t'           # 全部
pkg query -e '%a = 0' '%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t'  # 手动（prime-list）
pkg query -e '%a = 1' '%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t'  # 自动
```

格式符含义：`%n`=名称 `%v`=版本 `%o`=origin `%c`=comment `%a`=automatic flag `%sh`=人类可读大小 `%w`=主页 `%m`=维护者 `%t`=安装时间戳。

通过 `?filter=manual|automatic|all` 查询参数控制。

#### 2. 详情 `package_detail`

共 3 次调用（从原 5 次 TSV 调用降为 3 次，其中核心数据只需 1 次 JSON 调用）：

| 调用 | 命令 | 用途 |
|------|------|------|
| 主信息 | `pkg info -R --raw-format json-compact {name}` | 一次获取全部结构化数据：name/version/origin/prefix/comment/desc/maintainer/www/abi/arch/flatsize/timestamp/licenses/categories/deps |
| 补充字段 | `pkg query '%a\t%k\t%V\t%R' {name}` | raw manifest 不含的 automatic/locked/vital/repository |
| 反向依赖 | `pkg query '%rn\t%rv' {name}` | 被哪些包依赖（raw manifest 不含） |

**为何用 JSON？** `pkg info -R --raw-format json-compact` 输出完整的 UCL manifest 为 JSON 数组，天然处理 `%e`（description）等多行字段，无需手动处理 TSV 中换行符导致的解析错误。依赖关系以 `{"name": {"origin": "...", "version": "..."}}` 字典形式提供，结构清晰。`serde_json` 反序列化为 `RawManifest` 结构体，字段映射直接。

#### 3. 文件列表 `package_files`

```
pkg query '%Fp\t%Fu\t%Fg\t%Fm' {name}
```

格式符：`%Fp`=路径 `%Fu`=所有者 `%Fg`=用户组 `%Fm`=权限。使用 `splitn(4, '\t')` 防止路径中的特殊字符导致解析问题。

### 输入验证

包名验证正则 `^[a-zA-Z0-9_+.{}@-]+$`（允许 FreeBSD ports 中的合法包名字符），最大 256 字符。

### 前端

`web/js/pages/pkg.js` — 两个视图：

1. **列表页** (`/pkg`)：filter-group 切换（全部/手动/自动）+ 搜索框 + 表格。点击行跳转详情。
2. **详情页** (`/pkg/{name}`)：三个 tab：
   - **Info** — kv-table 展示基本信息 + 描述
   - **Dependencies** — 双栏：Depends On（依赖）/ Required By（被依赖），点击包名可跳转
   - **Files** — 文件列表（延迟加载，切 tab 时才请求 API）

## API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/pkg/packages` | 列出已安装包。`?filter=manual\|automatic\|all` |
| GET | `/api/pkg/packages/{name}` | 包详情（含依赖 + 反向依赖） |
| GET | `/api/pkg/packages/{name}/files` | 包文件列表 |

## 外部依赖

- 系统命令：`/usr/sbin/pkg`（`pkg query` 子命令）
- 无额外 Rust crate 依赖（使用已有的 `std::process::Command`、`regex`、`serde`、`axum`）

## 已知限制 / TODO

- **只读**：不支持 `pkg install`、`pkg upgrade`、`pkg delete`、`pkg lock` 等写操作。
- **无远程搜索**：不支持 `pkg search`（搜索远程仓库中可用的包）。
- **无更新检查**：不显示 `pkg version` 的可用更新信息。
- **无 audit**：不集成 `pkg audit` 安全漏洞检查。
- **文件列表无分页**：对于文件数极多的包（如 python），一次性返回全部文件。
