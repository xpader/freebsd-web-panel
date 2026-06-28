# 08 — 文件系统概览

## 概述

展示系统物理磁盘、挂载点列表、ZFS 存储池状态。数据来自 `geom`/`mount`/`df`/`zpool` 命令实时采集。作为「文件系统」主菜单下的第一个子页面。

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

### 前端 `web/js/pages/filesystem.js`

`renderFsOverview` 渲染三段：
1. **ZFS 存储池卡片**：每池一张卡，显示健康状态（ONLINE=绿/其他=红徽章）、容量、已用、碎片率、去重比、容量进度条（>80% 黄色/其他紫色）
2. **物理磁盘表格**：设备名、型号、容量、转速（`unknown` 显示 `SSD?`）
3. **挂载点表格**：设备、挂载点、类型徽章、总容量/已用/可用、使用率迷你进度条

### 菜单集成

`layout.js` 的「文件系统」主菜单现在有「概览」（`/filesystem`）+「ZFS」（`/zfs`）两个子项。点击「文件系统」主标签跳转到概览页。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/filesystem/overview` | 磁盘 + 挂载点 + ZFS 池概览 |

## 外部依赖

- 系统命令：`/sbin/geom`（disk list）、`/sbin/mount`、`/bin/df`、`/sbin/zpool`

## 已知限制

- 磁盘分区表（`gpart show`）未展示
- 磁盘温度（SMART）未采集
- 非 ZFS 的 UFS/MSDOSFS 挂载点也能显示（通过 mount+df），但无专门管理
- 列表无搜索/过滤（挂载点多时可考虑后续加分页）
