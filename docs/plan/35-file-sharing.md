# 文件共享 — SMB（Samba）

> 在文件管理下实现 SMB/CIFS 文件共享，通过面板管理 Samba 服务、共享目录、Samba 用户。

## 1. 目标

- 通过 Web 面板管理 SMB 共享（创建、编辑、删除共享目录）
- 管理 Samba 用户（添加、删除、改密）
- 管理 Samba 全局配置（workgroup、协议版本、日志等）
- 管理 Samba 服务（启动、停止、重启、开机自启）
- **明确约束：Samba 是第三方 pkg，不在 base system 中。面板需检测安装状态，未安装时引导安装。**

## 2. 背景与约束

### 为什么 SMB 必须依赖第三方包

FreeBSD base system **没有 SMB/CIFS 服务器**。唯一自带的 `mount_smbfs(8)` 是客户端，且只支持已废弃的 SMBv1。提供 SMB 共享必须安装 Samba：

| 组件 | 来源 | 包 |
|------|------|-----|
| `smbd` / `nmbd` / `winbindd` | Samba 套件 | `samba416`（或当前版本） |
| `pdbedit` / `smbpasswd` | Samba 套件 | 同上 |
| `smb4.conf` | 管理员创建 | `/usr/local/etc/smb4.conf` |
| rc.d 脚本 | Samba 套件自带 | `/usr/local/etc/rc.d/samba_server` |

**对比 NFS**：NFS 全套在 base system 中（`nfsd`、`mountd`、`rpcbind`），无需任何 pkg。NFS 可作为独立后续功能实现。

### 本面板的定位

面板**不内嵌 Samba**，而是作为 **Samba 的配置管理前端**：

```
用户 ──→ 面板 Web UI ──→ 生成/编辑 smb4.conf + pdbedit/smbpasswd 命令 ──→ Samba 服务
```

## 3. 架构设计

```
┌──────────────────────────────────────────────────────┐
│ Frontend (Vue 3)                                     │
│  /shares/smb               共享列表 + 增删改          │
│  /shares/smb/users         Samba 用户管理              │
│  /shares/smb/settings      全局配置                    │
└──────────────────────┬───────────────────────────────┘
                       │ HTTP API
┌──────────────────────▼───────────────────────────────┐
│ Backend (Rust)                                        │
│  handlers/smb.rs                                      │
│  ├── 安装检测: /usr/local/sbin/smbd 是否存在          │
│  ├── 配置管理: 解析/生成 /usr/local/etc/smb4.conf     │
│  ├── 共享 CRUD: 读 [share] 段 → 增删改 → 重载服务     │
│  ├── 用户管理: pdbedit -L / smbpasswd -a -s / -x      │
│  └── 服务管理: sysrc + service samba_server           │
└──────────────────────┬───────────────────────────────┘
                       │ spawn 命令
┌──────────────────────▼───────────────────────────────┐
│ System                                                │
│  /usr/local/sbin/smbd   (Samba 守护进程)              │
│  /usr/local/etc/smb4.conf  (配置文件)                 │
│  /usr/local/etc/rc.d/samba_server  (rc.d)             │
│  /var/db/samba4/passdb.tdb  (用户数据库)               │
└──────────────────────────────────────────────────────┘
```

## 4. 后端设计

### 4.1 状态检测（复用 Bhyve 模式）

与 Bhyve 的 `BhyveStatus` + `GET /api/bhyve/status` 模式完全一致：用一个结构体描述所有前置条件，前端根据它决定显示初始化引导还是正常页面。

```rust
const SAMBA_SMBD: &str = "/usr/local/sbin/smbd";
const SAMBA_CONF: &str = "/usr/local/etc/smb4.conf";
const SMBPASSWD: &str = "/usr/local/bin/smbpasswd";
const PDBEDIT: &str = "/usr/local/bin/pdbedit";
const SERVICE: &str = "/usr/sbin/service";
const RC_SERVICE_NAME: &str = "samba_server";

/// Samba 安装与运行状态（对标 bhyve::BhyveStatus）
#[derive(Debug, Clone, Serialize)]
pub struct SmbStatus {
    pub installed: bool,        // /usr/local/sbin/smbd 存在
    pub enabled: bool,          // rc.conf samba_server_enable == "YES"
    pub initialized: bool,      // /usr/local/etc/smb4.conf 存在（面板已初始化）
    pub service_running: bool,  // 读取 pid 文件 + kill(pid,0) 验证进程存在
    pub version: Option<String>, // smbd --version 解析
}

pub fn check_status() -> SmbStatus {
    let installed = Path::new(SAMBA_SMBD).exists();
    let rc = crate::sysrc::read_rcconf_files();
    let enabled = rc.get("samba_server_enable").map(|v| v == "YES").unwrap_or(false);
    let initialized = Path::new(SAMBA_CONF).exists();
    // service_running: 读取 /var/run/samba4/{smbd,nmbd}.pid + kill(pid,0)
    let service_running = installed && is_samba_running();
    // version: smbd --version → "Version 4.16.x"
    let version = if installed {
        crate::cmd::run_sync(SAMBA_SMBD, &["--version"]).ok()
            .and_then(|s| s.split_whitespace()
                .skip_while(|w| *w != "Version")
                .nth(1).map(|v| v.trim_end_matches(',').to_string()))
    } else { None };
    SmbStatus { installed, enabled, initialized, service_running, version }
}
```

**`needsInit` 判定逻辑**（前端计算，对标 Bhyve 的 `needsInit` computed）：

```
needsInit = !installed || !initialized
```

三个条件全部满足后才进入正常页面。所有数据查询 API（shares/users/config）在未初始化时返回 `ApiError::BadRequest("Samba not initialized")`，前端同时用 status 做双重防护。

### 4.2 初始化流程（对标 Bhyve 的 `POST /api/bhyve/init`）

初始化是一个**流式后台任务**（bgtask），前端通过 SSE 实时观看安装进度。三步流水线：

```
步骤 1/3: pkg install -y samba416
步骤 2/3: sysrc samba_server_enable=YES
步骤 3/3: 生成默认 smb4.conf（写入 /usr/local/etc/smb4.conf）+ 启动服务
```

```rust
pub async fn init(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    // 已初始化则拒绝
    let st = tokio::task::spawn_blocking(|| check_status()).await?;
    if st.installed && st.initialized {
        return Err(ApiError::Conflict("Samba already initialized"));
    }

    let id = bgtask::create("smb-init", "Install Samba");
    let tid = id.clone();
    let state2 = state.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        let exit = run_init_streaming(&tid).await;
        let ok = exit == 0;
        bgtask::set_status(&tid, if ok { TaskStatus::Done } else { TaskStatus::Failed }, Some(exit));
        audit::record(&state2, Some(&username), "POST", "/api/smb/init",
            if ok { 200 } else { 500 },
            Some(if ok { "Samba initialized".into() } else { "Samba init failed".into() }));
    });
    Ok(Json(json!({ "task_id": id })))
}
```

`run_init_streaming` 是三步顺序管线，每步 push_line 进度，非零退出则 abort（对标 Bhyve 的 `run_init_streaming`）：

```rust
async fn run_init_streaming(tid: &str) -> i32 {
    // 步骤 1/3: 安装 Samba
    bgtask::push_line(tid, "[1/3] Installing samba416...");
    let exit = bgtask::run_streaming_cmd(tid, "/usr/sbin/pkg",
        &["install", "-y", "samba416"]).await;
    if exit != 0 { /* set failed */ return exit; }

    // 步骤 2/3: 启用服务
    bgtask::push_line(tid, "[2/3] Enabling samba_server...");
    let exit = bgtask::run_streaming_cmd(tid, "/usr/sbin/sysrc",
        &["samba_server_enable=YES"]).await;
    if exit != 0 { return exit; }

    // 步骤 3/3: 生成默认配置 + 启动
    bgtask::push_line(tid, "[3/3] Generating smb4.conf...");
    match write_default_conf() {
        Ok(()) => {
            bgtask::run_streaming_cmd(tid, SERVICE, &[RC_SERVICE_NAME, "start"]).await;
            bgtask::push_line(tid, "Done.");
            0
        }
        Err(e) => { bgtask::push_line(tid, &format!("Error: {}", e)); 1 }
    }
}
```

`write_default_conf()` 生成最小可用配置（见 §7 的 `[global]` 段默认值）。

### 4.3 smb4.conf 配置解析与生成

smb4.conf 采用 **INI 格式**。面板自写轻量解析器。

#### 数据模型

```rust
/// 整个 smb4.conf 的结构化表示
#[derive(Serialize, Deserialize, Clone)]
pub struct SmbConfig {
    pub global: GlobalConfig,
    pub shares: Vec<SmbShare>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GlobalConfig {
    pub workgroup: String,           // 默认 "WORKGROUP"
    pub server_string: String,       // 默认 "FreeBSD Samba Server"
    pub server_role: String,         // 默认 "standalone"
    pub log_level: u8,              // 默认 1
    pub server_min_protocol: String, // 默认 "SMB2"
    pub map_to_guest: String,        // 默认 "Bad User"（允许 guest）
    pub passdb_backend: String,      // 默认 "tdbsam"
    pub dns_proxy: String,           // 默认 "no"
    pub load_printers: String,       // 默认 "no"
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SmbShare {
    pub name: String,              // 共享名，校验 ^[a-zA-Z0-9_.$-]+$
    pub comment: String,           // 描述
    pub path: String,              // 服务器上的绝对路径
    pub browseable: bool,          // 是否在网络浏览中可见
    pub writable: bool,            // 是否可写（read only 的反义）
    pub guest_ok: bool,            // 是否允许 guest 访问
    pub valid_users: Vec<String>,  // 允许的用户列表（空 = 所有 Samba 用户）
    pub create_mask: String,       // 新文件权限掩码，如 "0664"
    pub directory_mask: String,    // 新目录权限掩码，如 "0775"
}
```

#### 解析器逻辑

```
1. 读取 /usr/local/etc/smb4.conf（不存在则返回空默认配置）
2. 逐行扫描：
   - [section] → 开始一个新段（global 或 share）
   - key = value → 写入当前段
   - ; 或 # 开头 → 跳过注释
   - 空行 → 跳过
3. [global] 段 → GlobalConfig
4. 其他 [xxx] 段 → SmbShare（name=xxx）
   - yes/no 字符串 → bool
   - valid users 用空格分割 → Vec<String>
5. 仅映射面板支持的字段；不支持的字段不会进入结构化模型。
```

#### 生成器逻辑

```
1. 写 [global] 段，按固定字段顺序输出
2. 逐个 share 写 [name] 段
3. bool → yes/no
4. Vec<String> → 空格连接
5. 写临时文件 → atomic rename（与 pkg repos 相同模式）
```

#### 共享名校验

```rust
static RE_SHARE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z0-9_.$-]{1,64}$").unwrap()
});
```

#### 路径校验

校验路径为绝对路径且不含 NUL。当前不要求路径已存在或必须是目录，由 Samba 在实际访问时处理。

### 4.4 Samba 用户管理

Samba 用户**必须先存在于系统用户**（`/etc/passwd`），Samba 维护独立的密码数据库（`passdb.tdb`）。

#### 列出 Samba 用户

```sh
pdbedit -L                              # 简单列表：username:UID:fullname
pdbedit -L -v                           # 详细（含 SIDs）
```

面板解析 `pdbedit -L` 输出，返回 `{ username, uid }` 列表。

#### 添加 Samba 用户

```sh
printf '%s\n%s\n' "$password" "$password" | smbpasswd -a -s "$username"
```

**注意**：`smbpasswd -a` 需要交互式输入密码两次。`-s` 标志使其从 stdin 读取。

当前 `cmd::run` 设置 `stdin(Stdio::null())`，**需要新增一个支持 stdin 管道的命令运行函数**：

```rust
/// 同步执行命令，通过 stdin 传入数据。用于 smbpasswd -a -s。
pub fn run_sync_stdin(cmd: &str, args: &[&str], stdin_data: &[u8]) -> ApiResult<String> {
    use std::io::Write;
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(stdin_data)?;
    }
    let output = child.wait_with_output()?;
    output_ok(cmd, &output)
}
```

异步包装：

```rust
pub async fn run_stdin(cmd: &str, args: &[&str], stdin_data: Vec<u8>) -> ApiResult<String>
```

#### 删除 Samba 用户

```sh
pdbedit -x -u "$username"
```

#### 修改密码

与添加相同：`smbpasswd -a -s username`（覆盖已有密码）。

#### 前置条件校验

```rust
// 用户名校验：必须存在于 /etc/passwd
fn validate_system_user(username: &str) -> ApiResult<()> { ... }

// 用户名安全校验：防注入
static RE_USERNAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z0-9_.-]{1,32}$").unwrap()
});
```

### 4.5 服务管理

```rust
// 启用/禁用开机自启
sysrc::set("samba_server_enable", "YES"/"NO")

// 服务操作
cmd::run(SERVICE, &["samba_server", "start"/"stop"/"restart"/"reload"])

// 服务状态
cmd::run(SERVICE, &["samba_server", "status"])  // 返回值或 stderr 判断运行中
```

rc.conf 相关变量（Samba 套件提供）：
- `samba_server_enable` — 主开关
- `samba_server_config` — 配置文件路径（默认 `/usr/local/etc/smb4.conf`）

### 4.6 配置生效

修改 smb4.conf 后，需要重载 Samba 服务使配置生效：

```rust
// 重载共享配置，不中断现有连接
cmd::run(SERVICE, &["samba_server", "reload"]).await?;
```

## 5. API 设计

所有路由前缀 `/api/smb`，需要认证。

### 5.1 状态与初始化

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/smb/status` | 安装状态、服务运行状态、版本信息（对标 `GET /api/bhyve/status`） |
| POST | `/api/smb/init` | 初始化 Samba：安装 pkg + 启用服务 + 生成默认配置（流式后台任务，对标 `POST /api/bhyve/init`） |

`GET /api/smb/status` 响应：
```json
{
  "installed": false,
  "enabled": false,
  "initialized": false,
  "service_running": false,
  "version": null
}
```

`POST /api/smb/init` 响应：
```json
{
  "task_id": "smb-init-xxxx"
}
```
前端通过 `EventSource` 连接 `/api/tasks/{task_id}/stream` 实时观看安装日志（与 Bhyve init 完全相同的 SSE 模式）。

### 5.2 全局配置

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/smb/config` | 获取全局配置 |
| PUT | `/api/smb/config` | 更新全局配置（写入 smb4.conf 的 [global] 段，reload 服务） |

### 5.3 共享管理

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/smb/shares` | 列出所有共享 |
| POST | `/api/smb/shares` | 创建共享（校验共享名 + 路径，写配置，reload） |
| PUT | `/api/smb/shares/{name}` | 更新共享（校验，写配置，reload） |
| DELETE | `/api/smb/shares/{name}` | 删除共享（写配置，reload） |

创建/更新请求体：
```json
{
  "name": "public",
  "comment": "公共共享",
  "path": "/zroot/data/share",
  "browseable": true,
  "writable": true,
  "guest_ok": false,
  "valid_users": ["alice", "bob"],
  "create_mask": "0664",
  "directory_mask": "0775"
}
```

### 5.4 用户管理

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/smb/users` | 列出所有 Samba 用户 |
| POST | `/api/smb/users` | 添加 Samba 用户（需系统用户已存在） |
| DELETE | `/api/smb/users/{name}` | 删除 Samba 用户 |
| PUT | `/api/smb/users/{name}/password` | 修改密码 |
| GET | `/api/smb/sysusers` | 列出可作为 Samba 用户的系统用户（从 /etc/passwd 读取，过滤掉系统账户） |

创建用户请求体：
```json
{
  "username": "alice",
  "password": "secret123"
}
```

### 5.5 服务控制

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/smb/service/{action}` | start（同时 enable=YES）/ stop（同时 enable=NO）/ restart / reload |

## 6. 前端设计

### 6.1 菜单结构

在现有 `filesystem` 组下新增 SMB 子菜单：

```
文件系统 (filesystem)
├── 概览           /filesystem
├── 磁盘           /filesystem/disks
├── 文件管理       /filesystem/files
├── ZFS            /zfs/...
├── 文件共享       /shares/smb          ← 新增
│   ├── SMB 共享   /shares/smb          ← 默认页（共享列表）
│   ├── SMB 用户   /shares/smb/users
│   └── SMB 设置   /shares/smb/settings
```

`menu.js` 中 `filesystem` 组的 items 追加：

```js
{
  path: '/shares/smb',
  labelKey: 'nav.smb',
  icon: 'fa-solid fa-share-nodes',
  children: [
    { path: '/shares/smb', labelKey: 'nav.smbShares', icon: 'fa-solid fa-folder-tree' },
    { path: '/shares/smb/users', labelKey: 'nav.smbUsers', icon: 'fa-solid fa-user-key' },
    { path: '/shares/smb/settings', labelKey: 'nav.smbSettings', icon: 'fa-solid fa-gear' },
  ],
},
```

### 6.2 三态条件渲染（对标 BhyveVmsPage.vue）

SmbSharesPage.vue 采用与 `BhyveVmsPage.vue` 完全相同的三分支条件渲染模式：

```js
const smbStatus = ref(null);

const needsInit = computed(() => {
  if (!smbStatus.value) return false;
  const s = smbStatus.value;
  return !s.installed || !s.enabled || !s.initialized;
});

const initMessages = computed(() => {
  const s = smbStatus.value; const msgs = [];
  if (!s.installed)   msgs.push(t('smb.initMissingPkg'));
  if (!s.initialized) msgs.push(t('smb.initMissingConf'));
  return msgs;
});

onMounted(async () => {
  await loadStatus();              // GET /api/smb/status
  if (!needsInit.value) load();    // 仅就绪后才加载共享列表
  else loading.value = false;
});
```

模板三分支：

```html
<!-- 分支 A: needsInit — 显示初始化引导卡片 -->
<template v-if="needsInit">
  <div class="card init-card">
    <h3><i class="fa-solid fa-triangle-exclamation"></i> {{ t('smb.initRequired') }}</h3>
    <ul>
      <li v-for="msg in initMessages" :key="msg" class="warning">{{ msg }}</li>
    </ul>
    <button class="btn-primary" @click="router.push('/shares/smb/init')">
      <i class="fa-solid fa-rocket"></i> {{ t('smb.goInit') }}
    </button>
  </div>
</template>

<!-- 分支 B: 就绪 — 正常的共享列表 -->
<template v-else>
  <!-- 顶部创建/刷新按钮 + SmbStatusBar + 共享表格 -->
</template>
```

### 6.3 初始化页面（对标 BhyveInitPage.vue）

**SmbInitPage.vue** — 路由 `/shares/smb/init`。

与 Bhyve 的初始化向导不同，SMB 初始化**无需用户输入参数**（不需要选存储路径），因此更简单：只需一个"开始安装"按钮 + SSE 实时日志控制台。

```js
async function doInit() {
  const resp = await api.post('/api/smb/init');
  const taskId = resp.task_id;

  // SSE 流式输出（与 BhyveInitPage 完全相同）
  const es = new EventSource(`/api/tasks/${taskId}/stream?token=${auth.token}`);
  es.onmessage = (e) => {
    const data = JSON.parse(e.data);
    if (data.lines) taskOutput.value += data.lines.join('\n');
    if (data.status !== 'running') {
      es.close();
      finish(data.status === 'done');
    }
  };
}

function finish(ok) {
  if (ok) {
    toast.toast(t('smb.initSuccess'));
    setTimeout(() => router.push('/shares/smb'), 2000);
  } else {
    alert(t('smb.initFailed') + '\n' + taskOutput.value.split('\n').slice(-5).join('\n'));
  }
}
```

页面结构：
1. 标题 + 说明文字（"即将安装 Samba 并生成默认配置"）
2. "开始安装" 按钮 → `POST /api/smb/init` → 启动 SSE
3. 安装过程中：实时滚动日志（黑底终端样式，自动滚动到底部）
4. 成功后：toast → 2 秒后自动跳转回 `/shares/smb`
5. 失败后：alert 显示最后 5 行日志 + "重试" 按钮

### 6.4 功能页面

#### SmbSharesPage.vue（共享列表，初始化完成后显示）

- `.page-header` 右侧：创建共享和刷新按钮
- `SmbStatusBar`：基于通用 `StatusBar`，显示服务状态、版本、开机自启，并提供启停、重启和开机自启操作
- 表格列：共享名 | 路径 | 描述 | 可写 | Guest | 操作（编辑/删除）
- 创建/编辑：FormModal 对话框（共享名、路径、描述、browseable、writable、guest_ok、valid_users、create_mask、directory_mask）
- 路径选择：手动输入绝对路径
- 删除：useConfirm 确认 → API → toast 成功 / alert 失败

#### SmbUsersPage.vue（用户管理）

- 上半部分：Samba 用户列表（用户名 | UID | 改密 / 删除）
- 下半部分或对话框：添加用户
  - 选择系统用户（下拉，从 `/api/smb/sysusers` 获取）
  - 输入密码 + 确认密码
  - 已是 Samba 用户的系统用户不在候选列表中

#### SmbSettingsPage.vue（全局配置）

- 表单：workgroup、server_string、server_min_protocol（下拉：SMB2/SMB3）、map_to_guest、log_level
- 保存按钮 → PUT config → reload 服务
- 服务控制区：启动/停止/重启按钮（Start 联动 enable=YES，Stop 联动 enable=NO）

### 6.5 i18n

遵循项目翻译键命名规范：通用词复用 `common.*`，SMB 特有词新建 `nav.smbShares` / `nav.smbUsers` / `nav.smbSettings` 等。初始化相关键：`smb.initRequired` / `smb.initMissingPkg` / `smb.initMissingConf` / `smb.goInit` / `smb.initSuccess` / `smb.initFailed`。

## 7. smb4.conf 生成示例

面板管理的完整配置文件示例：

```ini
# Managed by FreeBSD Web Panel — do not edit manually

[global]
    workgroup = WORKGROUP
    server string = FreeBSD Samba Server
    server role = standalone
    map to guest = Bad User
    passdb backend = tdbsam
    logging = file
    log file = /var/log/samba4/log.%m
    max log size = 50
    server min protocol = SMB2
    dns proxy = no
    load printers = no

[public]
    comment = 公共共享
    path = /zroot/data/public
    browseable = yes
    writable = yes
    guest ok = no
    valid users = alice bob
    create mask = 0664
    directory mask = 0775

[homes]
    comment = Home Directories
    path = /usr/home
    browseable = no
    writable = yes
    guest ok = no
```

## 8. 安全考虑

1. **密码传输**：面板是纯 HTTP，Samba 密码以明文传到面板 API，由面板通过 stdin 传给 `smbpasswd`。面板进程内存中短暂持有密码。生产环境应前置 TLS 反向代理。
2. **路径限制**：共享路径复用 `normalize()` 校验，禁止 `..` 穿越。面板不做路径白名单（管理员自行决定共享哪些目录）。
3. **命令注入**：所有用户输入（共享名、用户名、密码）通过 `Command::new().arg()` 传递，不拼接 shell。共享名/用户名用正则白名单校验。
4. **smb4.conf 写入**：atomic write（tmp + rename），避免部分写入导致 Samba 加载失败。
5. **权限**：面板以 root 运行，可以直接执行 `smbpasswd`、编辑 smb4.conf。

## 9. 外部依赖

| 依赖 | 类型 | 说明 |
|------|------|------|
| samba416（或当前版本） | pkg | 提供 smbd、nmbd、pdbedit、smbpasswd、rc.d 脚本 |
| `cmd::run_stdin()` | 新增内部函数 | 在 `src/cmd.rs` 中新增，支持 stdin 管道（用于 smbpasswd -a -s） |
| INI 解析/生成 | 自写 | 轻量，无外部 crate |

## 10. 配置项

无新增 fwp.toml 配置项。Samba 配置全部存储在 `/usr/local/etc/smb4.conf`。

## 11. 实现清单

### 后端（Rust）

1. `src/cmd.rs` — 新增 `run_sync_stdin()` + `run_stdin()` 异步包装（用于 smbpasswd -a -s）
2. `src/handlers/smb.rs` — 新建 handler 模块：
   - `check_status()` + `SmbStatus` 结构体 — 状态检测（对标 `bhyve::check_status`）
   - `status` — `GET /api/smb/status`
   - `init` + `run_init_streaming()` — `POST /api/smb/init` 流式初始化任务（对标 `POST /api/bhyve/init`）
   - `write_default_conf()` — 生成默认 smb4.conf
   - `parse_conf()` / `generate_conf()` — smb4.conf 读写
   - `get_config` / `update_config` — 全局配置 CRUD
   - `list_shares` / `create_share` / `update_share` / `delete_share` — 共享 CRUD
   - `list_users` / `create_user` / `delete_user` / `change_password` / `list_sysusers` — 用户管理
   - `service_control` — 服务 start/stop/restart/reload
3. `src/handlers/mod.rs` — 新增 `pub mod smb;`
4. `src/app.rs` — 注册所有 SMB 路由

### 前端（Vue）

5. `frontend/src/pages/SmbSharesPage.vue` — 共享列表（含三态条件渲染：needsInit 引导 / 正常表格）
6. `frontend/src/pages/SmbInitPage.vue` — 初始化向导（SSE 实时日志，对标 `BhyveInitPage.vue`）
7. `frontend/src/pages/SmbUsersPage.vue` — 用户管理
8. `frontend/src/pages/SmbSettingsPage.vue` — 全局配置 + 服务控制
9. `frontend/src/lib/menu.js` — filesystem 组下新增 SMB 子菜单
10. `frontend/src/router/index.js` — 注册四条新路由（含 `/shares/smb/init`）
11. `frontend/src/i18n/translations.js` — 新增翻译键

### 文档

12. `docs/impl/31-smb.md` — 实现文档（实现后编写）

## 12. 已知限制

1. **不支持 Windows ACL**：面板管理的 create_mask / directory_mask 是简单的 Unix 权限掩码，不处理 Windows ACL（NFSv4 ACL / NTACL）。
2. **不支持域控/AD 集成**：仅管理 standalone 模式（`server role = standalone`），不处理 Active Directory 域加入、Kerberos 等。
3. **不管理打印机共享**：`load printers = no` 默认关闭，面板不提供打印机共享管理。
4. **单 conf 文件**：面板完全管理 `/usr/local/etc/smb4.conf`，不支持 `include` 指令拆分多文件。保留面板不识别的 `extra` 字段避免丢失用户自定义配置。
5. **密码明文**：面板 HTTP API 传输 Samba 密码为明文（与面板自身的 session token 一样），生产环境需 TLS 反代。
6. **无实时连接监控**：不展示当前 SMB 连接/会话（`smbstatus`）。可作为后续增强。
