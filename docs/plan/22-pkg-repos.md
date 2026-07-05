# PKG 软件源配置管理

> 状态：**已实现** — 见 `docs/impl/22-pkg-repos.md`

## 背景

pkg 包管理模块（`docs/impl/21-pkg.md`）已完成包列表、详情、搜索、安装、删除。本模块新增**软件源（Repository）配置管理**，让用户通过 Web 面板查看和管理 pkg 仓库源。

## FreeBSD pkg 软件源配置机制

### 配置目录层级

pkg 按以下顺序搜索 `.conf` 文件（UCL 格式），后者覆盖前者：

| 目录 | 用途 |
|---|---|
| `/etc/pkg/` | 系统默认源（如 `FreeBSD.conf`），由 base 系统提供 |
| `/usr/local/etc/pkg/repos/` | 用户自定义源，**面板管理的目标目录** |

`REPOS_DIR` 默认值为 `["/etc/pkg/", "/usr/local/etc/pkg/repos/"]`，可在 `/usr/local/etc/pkg.conf` 中修改。

### 配置文件格式（UCL）

每个 `.conf` 文件可包含一个或多个仓库定义，格式如下：

```ucl
repo-name: {
  url: "pkg+https://pkg.freebsd.org/${ABI}/quarterly",
  mirror_type: "srv",          # NONE | HTTP | SRV
  signature_type: "fingerprints", # NONE | PUBKEY | FINGERPRINTS
  fingerprints: "/usr/share/keys/pkg",
  enabled: yes,
  priority: 0,
  ip_version: 0,               # 0=默认, 4=仅IPv4, 6=仅IPv6
}
```

**字段说明**：

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `url` | string | — (必填) | 仓库 URL，支持 `${ABI}` `${VERSION_MAJOR}` 等变量展开 |
| `enabled` | bool | yes | 是否启用 |
| `mirror_type` | string | NONE | 镜像发现方式：NONE/HTTP/SRV |
| `signature_type` | string | NONE | 签名验证：NONE/PUBKEY/FINGERPRINTS |
| `pubkey` | string | — | 公钥路径（signature_type=PUBKEY 时） |
| `fingerprints` | string | — | 指纹目录（signature_type=FINGERPRINTS 时） |
| `priority` | int | 0 | 优先级，**值越大越优先** |
| `ip_version` | int | 0 | IP 版本限制 |
| `env` | object | — | 传递给 fetch 的环境变量 |

### URL 变量

| 变量 | 展开值 |
|---|---|
| `${ABI}` | 如 `FreeBSD:15:amd64` |
| `${OSNAME}` | 如 `FreeBSD` |
| `${RELEASE}` | 如 `15.1-RELEASE` |
| `${VERSION_MAJOR}` | 如 `15` |
| `${VERSION_MINOR}` | 如 `1` |
| `${OSVERSION}` | `__FreeBSD_version` |
| `${ARCH}` | 如 `amd64` |

### 覆盖机制

同一 repo name 出现在多个文件中时，后面的文件覆盖前面的。常见用法：

```ucl
# /usr/local/etc/pkg/repos/FreeBSD.conf — 禁用默认源
FreeBSD: { enabled: no }
```

### 镜像类型

| 类型 | 说明 | URL scheme 要求 |
|---|---|---|
| `NONE` | 直连 | http/https/file/ssh/tcp |
| `SRV` | DNS SRV 记录自动发现镜像 | pkg+http / pkg+https |
| `HTTP` | URL 返回镜像列表文档 | http/https |

## 设计决策（已实现）

### 1. 按文件分组，同名文件合并

与系统配置（`/etc/pkg/FreeBSD.conf` 含多个 repo）保持一致。同名系统文件与用户文件合并展示为一个 Custom 文件。详见实现文档。

### 2. 自研 UCL 解析器

两阶段行式解析器（Phase 1 花括号深度拆块 → Phase 2 key-value 解析），不依赖 `pkg -vv` 或 C 库。

### 3. 最小差异写入

覆盖文件只写与系统原始配置不同的字段。详见实现文档。

### 4. 预设镜像模板

提供 FreeBSD 官方 latest/quarterly、中科大（ustc.conf）、清华（tuna.conf）快捷模板，一键填充仓库名、URL 和文件名。
