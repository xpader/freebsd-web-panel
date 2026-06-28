// Application entry — wires routes and starts the router.

import { startRouter, defineRoute } from './router.js';
import { clearToken } from './api.js';
import { renderLogin, renderSetup } from './pages/auth.js';
import { renderDashboard } from './pages/dashboard.js';
import { renderUsers } from './pages/users.js';
import { renderAudit } from './pages/audit.js';
import { renderMonitorCpu, renderMonitorMemory, renderMonitorTemp } from './pages/monitor.js';
import { makePlannedPage } from './pages/planned.js';

// Auth routes.
defineRoute('/login', renderLogin);
defineRoute('/setup', renderSetup);

// Core routes.
defineRoute('/dashboard', renderDashboard);
defineRoute('/users', renderUsers);
defineRoute('/audit', renderAudit);

// Monitor routes.
defineRoute('/monitor', renderMonitorCpu);
defineRoute('/monitor/memory', renderMonitorMemory);
defineRoute('/monitor/temp', renderMonitorTemp);
// Module placeholder routes.
defineRoute('/sysctl', makePlannedPage('/sysctl', 'Sysctl 系统参数', '动态内核参数 (sysctl) 管理', '通过 sysctl 命令读写运行时参数，并持久化到 /etc/sysctl.conf。'));
defineRoute('/rcconf', makePlannedPage('/rcconf', 'RC 配置', 'rc.conf 系统与服务启动配置', '通过 sysrc 管理 rc.conf 键值，按功能分类展示。'));
defineRoute('/network', makePlannedPage('/network', '网络', '网络接口、IP、路由管理', '解析 ifconfig 输出，管理接口 IP/别名/路由并持久化。'));
defineRoute('/services', makePlannedPage('/services', '服务', 'rc.d 服务管理', '列出可用/已启用服务，执行 start/stop/restart。'));
defineRoute('/pf', makePlannedPage('/pf', '防火墙 (PF)', 'Packet Filter 规则与状态', '查询 pfctl 状态/规则/表，编辑 /etc/pf.conf。'));
defineRoute('/jails', makePlannedPage('/jails', 'Jail 容器', 'FreeBSD Jail 原生管理（libjail，不依赖第三方工具）', '通过 libjail FFI (jailparam_*) 管理生命周期，解析 /etc/jail.conf。'));
defineRoute('/bhyve', makePlannedPage('/bhyve', 'Bhyve 虚拟机', '基于 vm-bhyve 的虚拟机管理', '封装 vm-bhyve CLI，管理 VM 创建/启动/快照/控制台。'));
defineRoute('/zfs', makePlannedPage('/zfs', 'ZFS 文件系统', 'ZFS Pool / Dataset / 快照管理', '通过 zfs/zpool 命令管理 pool、dataset、快照、克隆、scrub。'));

// Global logout handler.
window.__fwpLogout = async () => {
  try { await import('./api.js').then(({ api }) => api.post('/api/auth/logout')); } catch {}
  clearToken();
  location.hash = '#/login';
};

startRouter();
