// Application entry — wires routes and starts the router.

import { startRouter, defineRoute, reload } from './router.js';
import { clearToken } from './api.js';
import { initI18n } from './i18n/index.js';
import { renderLogin, renderSetup } from './pages/auth.js';
import { renderDashboard } from './pages/dashboard.js';
import { renderUsers } from './pages/users.js';
import { renderAudit } from './pages/audit.js';
import { renderMonitorCpu, renderMonitorMemory, renderMonitorTemp, renderMonitorNetwork } from './pages/monitor.js';
import { makePlannedPage } from './pages/planned.js';
import { renderFsOverview } from './pages/filesystem.js';
import { renderDisks } from './pages/disks.js';
import { renderFiles } from './pages/files.js';
import { renderZfsPools, renderZfsPoolDetail, renderZfsDatasets, renderZfsSnapshots } from './pages/zfs.js';
import { renderSysUsers, renderSysGroups } from './pages/accounts.js';
import { renderRcconf } from './pages/rcconf.js';
import { renderTerminal } from './pages/terminal.js';

// Auth routes.
defineRoute('/login', renderLogin);
defineRoute('/setup', renderSetup);

// Core routes.
defineRoute('/dashboard', renderDashboard);
defineRoute('/shell', renderTerminal);
defineRoute('/users', renderUsers);
defineRoute('/audit', renderAudit);

// Monitor routes.
defineRoute('/monitor', renderMonitorCpu);
defineRoute('/monitor/memory', renderMonitorMemory);
defineRoute('/monitor/temp', renderMonitorTemp);
defineRoute('/monitor/network', renderMonitorNetwork);
defineRoute('/filesystem', renderFsOverview);
defineRoute('/filesystem/disks', renderDisks);
defineRoute('/filesystem/files', renderFiles);
defineRoute('/sysctl', makePlannedPage({ key: 'sysctl', labelKey: 'nav.sysctl' }));
defineRoute('/rcconf', renderRcconf);
defineRoute('/network', makePlannedPage({ key: 'network', labelKey: 'common.network' }));
defineRoute('/services', makePlannedPage({ key: 'services', labelKey: 'nav.services' }));
// System accounts routes.
defineRoute('/accounts/users', renderSysUsers);
defineRoute('/accounts/groups', renderSysGroups);
defineRoute('/pf', makePlannedPage({ key: 'pf', labelKey: 'nav.pf' }));
defineRoute('/jails', makePlannedPage({ key: 'jails', labelKey: 'nav.jails' }));
// ZFS routes.
defineRoute('/zfs/pools', renderZfsPools);
defineRoute('/zfs/pools/', renderZfsPoolDetail);
defineRoute('/zfs/datasets', renderZfsDatasets);
defineRoute('/zfs/snapshots', renderZfsSnapshots);

// Global logout handler.
window.__fwpLogout = async () => {
  try { await import('./api.js').then(({ api }) => api.post('/api/auth/logout')); } catch {}
  clearToken();
  location.hash = '#/login';
};

// Boot: init i18n first so the first render has translations ready,
// then re-render on every language switch.
initI18n().then(() => {
  window.addEventListener('fwp:langchange', reload);
  startRouter();
});
