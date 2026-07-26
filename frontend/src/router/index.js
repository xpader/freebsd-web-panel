// Vue Router configuration — hash mode with auth guards.
// All page components are lazy-loaded (dynamic import) so Vite splits
// them into separate chunks, reducing initial bundle size.

import { createRouter, createWebHashHistory } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

import AppLayout from '../components/layout/AppLayout.vue';

const routes = [
  { path: '/login', name: 'login', component: () => import('../pages/LoginPage.vue'), meta: { auth: false } },
  { path: '/setup', name: 'setup', component: () => import('../pages/SetupPage.vue'), meta: { auth: false } },
  {
    path: '/',
    component: AppLayout,
    meta: { auth: true },
    children: [
      { path: '', redirect: '/dashboard' },
      { path: 'dashboard', name: 'dashboard', component: () => import('../pages/DashboardPage.vue') },
      { path: 'shell', name: 'shell', component: () => import('../pages/ShellPage.vue') },
      { path: 'dialog-demo', name: 'dialog-demo', component: () => import('../pages/DialogDemoPage.vue') },
      { path: 'users', name: 'users', component: () => import('../pages/UsersPage.vue') },
      { path: 'audit', name: 'audit', component: () => import('../pages/AuditPage.vue') },
      { path: 'monitor', name: 'monitor-cpu', component: () => import('../pages/MonitorCpuPage.vue') },
      { path: 'monitor/memory', name: 'monitor-memory', component: () => import('../pages/MonitorMemoryPage.vue') },
      { path: 'monitor/network', name: 'monitor-network', component: () => import('../pages/MonitorNetworkPage.vue') },
      { path: 'monitor/fwp', name: 'monitor-fwp', component: () => import('../pages/MonitorFwpPage.vue') },
      { path: 'mail', name: 'mail', component: () => import('../pages/MailPage.vue') },
      { path: 'filesystem', name: 'filesystem', component: () => import('../pages/FilesystemOverviewPage.vue') },
      { path: 'filesystem/disks', name: 'disks', component: () => import('../pages/DisksPage.vue') },
      { path: 'filesystem/files', name: 'files', component: () => import('../pages/FilesPage.vue') },
      { path: 'sysctl', name: 'sysctl', component: () => import('../pages/SysctlPage.vue') },
      { path: 'rcconf', name: 'rcconf', component: () => import('../pages/RcconfPage.vue') },
      { path: 'cron', name: 'cron', component: () => import('../pages/CronPage.vue') },
      { path: 'network', name: 'network', component: () => import('../pages/NetworkPage.vue') },
      { path: 'network/dns', name: 'dns', component: () => import('../pages/DnsPage.vue') },
      { path: 'network/routes', name: 'static-routes', component: () => import('../pages/StaticRoutesPage.vue') },
      { path: 'services', name: 'services', component: () => import('../pages/ServicesPage.vue') },
      { path: 'pf', redirect: '/firewall/rules' },
      { path: 'firewall', redirect: '/firewall/rules' },
      { path: 'firewall/rules', name: 'firewall-rules', component: () => import('../pages/FirewallRulesPage.vue') },
      { path: 'firewall/nat', name: 'firewall-nat', component: () => import('../pages/FirewallNatPage.vue') },
      { path: 'firewall/tables', name: 'firewall-tables', component: () => import('../pages/FirewallTablesPage.vue') },
      { path: 'firewall/settings', name: 'firewall-settings', component: () => import('../pages/FirewallSettingsPage.vue') },
      { path: 'bhyve', redirect: '/bhyve/vms' },
      { path: 'bhyve/vms', name: 'bhyve-vms', component: () => import('../pages/BhyveVmsPage.vue') },
      { path: 'bhyve/create', name: 'bhyve-create', component: () => import('../pages/BhyveCreatePage.vue') },
      { path: 'bhyve/detail/:name', name: 'bhyve-detail', component: () => import('../pages/BhyveDetailPage.vue') },
      { path: 'bhyve/edit/:name', name: 'bhyve-edit', component: () => import('../pages/BhyveEditPage.vue') },
      { path: 'bhyve/console/:name', name: 'bhyve-console', component: () => import('../pages/BhyveConsolePage.vue') },
      { path: 'bhyve/vnc/:name', name: 'bhyve-vnc', component: () => import('../pages/BhyveVncPage.vue') },
      { path: 'bhyve/images', name: 'bhyve-images', component: () => import('../pages/BhyveImagesPage.vue') },
      { path: 'bhyve/switches', name: 'bhyve-switches', component: () => import('../pages/BhyveSwitchesPage.vue') },
      { path: 'bhyve/switches/:name', name: 'bhyve-switch-detail', component: () => import('../pages/BhyveSwitchDetailPage.vue') },
      { path: 'bhyve/datastores', name: 'bhyve-datastores', component: () => import('../pages/BhyveDatastoresPage.vue') },
      { path: 'bhyve/isos', name: 'bhyve-isos', component: () => import('../pages/BhyveIsosPage.vue') },
      { path: 'bhyve/init', name: 'bhyve-init', component: () => import('../pages/BhyveInitPage.vue') },
      { path: 'accounts/users', name: 'accounts-users', component: () => import('../pages/AccountsUsersPage.vue') },
      { path: 'accounts/groups', name: 'accounts-groups', component: () => import('../pages/AccountsGroupsPage.vue') },
      { path: 'jails/running', name: 'jails-list', component: () => import('../pages/JailsListPage.vue') },
      { path: 'jails/create', name: 'jail-create', component: () => import('../pages/JailCreatePage.vue') },
      { path: 'jails/detail/:name', name: 'jail-detail', component: () => import('../pages/JailDetailPage.vue') },
      { path: 'jails/edit/:name', name: 'jail-edit', component: () => import('../pages/JailEditPage.vue') },
      { path: 'jails/terminal/:name', name: 'jail-terminal', component: () => import('../pages/JailTerminalPage.vue') },
      { path: 'jails/bases', name: 'jail-bases', component: () => import('../pages/JailBasesPage.vue') },
      { path: 'jails/defaults', name: 'jail-defaults', component: () => import('../pages/JailDefaultsPage.vue') },
      { path: 'zfs/pools', name: 'zfs-pools', component: () => import('../pages/ZfsPoolsPage.vue') },
      { path: 'zfs/pools/:name', name: 'zfs-pool-detail', component: () => import('../pages/ZfsPoolDetailPage.vue') },
      { path: 'zfs/datasets', name: 'zfs-datasets', component: () => import('../pages/ZfsDatasetsPage.vue') },
      { path: 'zfs/snapshots', name: 'zfs-snapshots', component: () => import('../pages/ZfsSnapshotsPage.vue') },
      { path: 'pkg', name: 'packages', component: () => import('../pages/PackagesPage.vue') },
      { path: 'pkg/repos', name: 'pkg-repos', component: () => import('../pages/PkgReposPage.vue') },
      { path: 'pkg/:name', name: 'package-detail', component: () => import('../pages/PackageDetailPage.vue') },
      { path: 'shares/smb', name: 'smb-shares', component: () => import('../pages/SmbSharesPage.vue') },
      { path: 'shares/smb/init', name: 'smb-init', component: () => import('../pages/SmbInitPage.vue') },
      { path: 'shares/smb/users', name: 'smb-users', component: () => import('../pages/SmbUsersPage.vue') },
      { path: 'shares/smb/settings', name: 'smb-settings', component: () => import('../pages/SmbSettingsPage.vue') },
    ],
  },
  { path: '/:pathMatch(.*)*', name: 'not-found', component: () => import('../pages/NotFoundPage.vue') },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

// Auth guard — redirect unauthenticated users to login/setup.
router.beforeEach(async (to) => {
  const auth = useAuthStore();

  // Auth pages: redirect logged-in users to dashboard.
  if (to.meta.auth === false) {
    if (auth.isLoggedIn()) return { name: 'dashboard' };
    return true;
  }

  // Protected pages: require login.
  if (!auth.isLoggedIn()) {
    const setup = await auth.resolveNeedsSetup();
    return setup ? { name: 'setup' } : { name: 'login' };
  }

  // Load user info once via Pinia (shared across all components).
  if (!auth.user) await auth.fetchUser();

  return true;
});

export default router;
