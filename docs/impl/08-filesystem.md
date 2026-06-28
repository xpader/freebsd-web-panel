# 08 — 文件系统

## 概述

「文件系统」主菜单下含两个子页面：

- **概览**（`/filesystem`）：物理磁盘、挂载点、ZFS 存储池概要
- **磁盘**（`/filesystem/disks`）：各磁盘详细参数 + 分区表

数据来自 `geom`/`mount`/`df`/`zpool` 命令实时采集。

## 实现细节

### 后端 `src/handlers/filesystem.rs`

`GET /api/filesystem/overview` 返回三部分数据：

**物理磁盘** — `geom disk list` 解析：
- 遍历输出行，每行先 trim 再剥离 `N. ` 数字前缀（`geom` 输出格式为 `1. Name: ada0`）
- 提取 `Name:`/`Mediasize:`/`descr:`/`rotationrate:` 字段
- 跳过 `Mediasize = 0` 的设备（如光驱 cd0）
- 每个 Disk：name、descr（型号）、size_bytes、rotation_rate

**挂载点** — `mount` + `df -k` 联合解析：
- `mount` 输出格式 `device on /mountpoint (fstype, options)` → 拆分提取 device/mountpoint/fstype/options
- `df -k`（1K-blocks）补充 size/used/available/capacity（按 mountpoint 匹配，`× 1024` 转字节）

**ZFS 存储池** — `zpool list -H -p` 解析：
- 机器可读格式（tab 分隔，精确数值）：NAME SIZE ALLOC FREE CKPOINT EXPANDSZ FRAG CAP DEDUP HEALTH ALTROOT
- 按列索引提取

### 磁盘详情 `GET /api/filesystem/disks`

`list_disk_details()` 合并两个 `geom` 命令的输出，构建每个磁盘的 `DiskDetail`（含分区表）：

**基础字段** — `geom disk list` 解析（`HashMap<String, DiskDetail>`，以磁盘名为键）：
- 同样剥离 `N. ` 前缀，逐行提取 `Name:`/`Mediasize:`/`Sectorsize:`/`Mode:`/`descr:`/`lunid:`/`ident:`/`rotationrate:`/`fwsectors:`/`fwheads:`
- 跳过 `Mediasize = 0` 的设备

**分区表** — `geom part list` 解析（`parse_geom_part()`）：
- 每个 `geom` 块以 `Geom name: <disk>` 开头，含顶层元数据（`scheme:`/`state:`/`first:`/`last:`/`entries:`）+ `Providers:`（分区列表）+ `Consumers:` 段
- 分区提供者行格式 `N. Name: ada0p1`，后跟 `Mediasize:`/`Sectorsize:`/`type:`/`label:`/`index:`/`start:`/`end:`/`offset:`/`rawuuid:`
- 状态机跟踪当前 geom 块、是否在 `Providers:` 段内；遇新块/`Consumers:` 时 flush 当前分区挂到对应磁盘
- 顶层元数据写入磁盘的 `scheme`/`state`/`first`/`last`/`entries`

最终 `disks.into_values()` 按名称排序返回（ada0, ada1, da0, …）。

### 前端 `web/js/pages/filesystem.js`

`renderFsOverview` 渲染三段：
1. **ZFS 存储池卡片**：每池一张卡，显示健康状态（ONLINE=绿/其他=红徽章）、容量、已用、碎片率、去重比、容量进度条（>80% 黄色/其他紫色）
2. **物理磁盘表格**：设备名、型号、容量、转速（`unknown` 显示 `SSD?`）
3. **挂载点表格**：设备、挂载点、类型徽章、总容量/已用/可用、使用率迷你进度条

### 前端 `web/js/pages/disks.js`

`renderDisks`（`/filesystem/disks`）渲染每张磁盘卡片：
- 头部：磁盘名 + 型号 + 分区方案徽章 + 状态徽章 + 总容量
- 参数网格（`stat-grid`）：设备路径、型号、总容量、扇区大小、序列号(ident)、LUN ID、转速、访问模式、固件扇区/磁头、GPT 元数据
- 已分配进度条（分区大小之和 / 总容量）
- 分区表：设备/类型/标签/大小/起止扇区/UUID。UUID 截断显示前 8 位，悬停（`.uuid-tip` CSS 伪元素）显示完整 UUID，点击复制到剪贴板并 toast 提示

### 菜单集成

`layout.js` 的「文件系统」主菜单含：「概览」（`/filesystem`）+「磁盘」（`/filesystem/disks`）+「ZFS」（`/zfs`）三个子项。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/filesystem/overview` | 磁盘 + 挂载点 + ZFS 池概览 |
| GET | `/api/filesystem/disks` | 各磁盘详细参数 + 分区表 |

## 外部依赖

- 系统命令：`/sbin/geom`（disk list、part list）、`/sbin/mount`、`/bin/df`、`/sbin/zpool`

## 已知限制

- 磁盘温度（SMART）未采集
- 非 ZFS 的 UFS/MSDOSFS 挂载点也能显示（通过 mount+df），但无专门管理
- 列表无搜索/过滤（挂载点多时可考虑后续加分页）
