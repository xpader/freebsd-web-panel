# FreeBSD Web Panel — 计划文档索引

> 技术架构与实施计划。**请审阅后决定是否实施。**

## 文档列表

| 文档 | 内容 |
|---|---|
| [00-overview.md](00-overview.md) | **项目总览 + 技术架构决策**（后端 Rust+Axum、前端原生 SPA、Jail libjail FFI、Bhyve vm-bhyve 封装、ZFS 命令封装、部署形态、安全模型、非目标） |
| [10-jail.md](10-jail.md) | Jail 模块设计（jail.conf 解析器、libjail FFI 绑定 `jailparam_*`、API、里程碑） |
| [20-bhyve.md](20-bhyve.md) | Bhyve 模块设计（vm-bhyve 命令映射、输出解析、VM/交换机/ISO API） |
| [30-zfs.md](30-zfs.md) | ZFS 模块设计（pool/dataset/snapshot、`-H -p` 解析、send/receive、危险操作保护） |
| [40-system.md](40-system.md) | 系统配置设计（sysctl、rc.conf、网络、服务、pf 防火墙） |
| [50-frontend.md](50-frontend.md) | 前端设计（原生 SPA 无构建、目录结构、页面规划、xterm 控制台） |
| [60-security-auth.md](60-security-auth.md) | 安全与认证（TLS、token/PAM 认证、审计日志、输入校验防注入） |
| [70-task-queue.md](70-task-queue.md) | 长任务队列（ISO 下载、scrub、send/receive 的异步化与进度） |
| [80-roadmap.md](80-roadmap.md) | **分阶段实施路线图**（Phase 0–8，验收标准，依赖关系图） |
| [90-monitoring.md](90-monitoring.md) | 监控模块设计（时序采集、SQLite 存储、图表、告警规则、通知渠道） |
| [22-pkg-repos.md](22-pkg-repos.md) | PKG 软件源配置管理设计（仓库 CRUD、UCL 配置、镜像模板、`pkg update` 后台任务） |
| [18-firewall.md](18-firewall.md) | **防火墙设计（双驱动 ipfw/pf、驱动切换、结构化规则 CRUD、黑白名单模式、rc.conf 初始化）** |
| [19-nat.md](19-nat.md) | **NAT 与端口转发设计（SNAT/DNAT、独立模型 + 嵌入式生成、复用 staging/apply/防锁死链路）** |
| [35-file-sharing.md](35-file-sharing.md) | **SMB 文件共享设计（Samba 依赖检测、smb4.conf 解析/生成、共享 CRUD、Samba 用户管理、服务控制）** |

## 架构一图速览

```
浏览器 (原生 SPA, Pico.css)
   │ HTTPS + Token
   ▼
Rust 单二进制 (axum + tokio)
   ├── sysctl / rc.conf / 网络 / 服务 / pf   → 子进程封装 (sysctl/sysrc/ifconfig/pfctl)
   ├── Jail   → libjail FFI (jailparam_*) + /etc/jail.conf 解析   [不依赖第三方工具]
   ├── Bhyve  → vm-bhyve CLI 封装
   └── ZFS    → zfs/zpool CLI (-H -p 机器可读) 封装
```

## 关键决策摘要

1. **后端 Rust + Axum**：单二进制、FFI 调 libjail、强类型、tokio 异步。
2. **前端 Vue 3 SPA**：Composition API + Vite 构建，由 rust-embed 内嵌进二进制。
3. **Jail 走 libjail 系统调用**（`jailparam_set/get/all` + `jail_remove`）+ 自写 jail.conf 解析器，**零第三方工具依赖**——满足你的约束。
4. **Bhyve 走 vm-bhyve**：按你的要求复用 1.7.3。
5. **防火墙双驱动**：ipfw / pf 可选可切换，结构化规则 CRUD，黑白名单模式（[详见 18-firewall.md](18-firewall.md)）。
6. **安全**：自带用户体系（Argon2id + session token）、审计日志、子进程参数白名单防注入。
7. **分 8 阶段交付**，Phase 0 完成后各子系统可并行开发。

---

**下一步：请审阅以上计划。确认后我将从 Phase 0（项目骨架）开始实施。** 如需调整范围、技术选型或优先级，请指出。
