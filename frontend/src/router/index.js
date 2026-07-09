// Vue Router configuration — hash mode with auth guards.

import { createRouter, createWebHashHistory } from 'vue-router';
import { useAuthStore } from '../stores/auth.js';

import AppLayout from '../components/layout/AppLayout.vue';
import LoginPage from '../pages/LoginPage.vue';
import SetupPage from '../pages/SetupPage.vue';
import DashboardPage from '../pages/DashboardPage.vue';
import UsersPage from '../pages/UsersPage.vue';
import AuditPage from '../pages/AuditPage.vue';
import ShellPage from '../pages/ShellPage.vue';
import MonitorCpuPage from '../pages/MonitorCpuPage.vue';
import MonitorMemoryPage from '../pages/MonitorMemoryPage.vue';
import MonitorNetworkPage from '../pages/MonitorNetworkPage.vue';
import FilesystemOverviewPage from '../pages/FilesystemOverviewPage.vue';
import DisksPage from '../pages/DisksPage.vue';
import FilesPage from '../pages/FilesPage.vue';
import SysctlPage from '../pages/SysctlPage.vue';
import RcconfPage from '../pages/RcconfPage.vue';
import CronPage from '../pages/CronPage.vue';
import NetworkPage from '../pages/NetworkPage.vue';
import DnsPage from '../pages/DnsPage.vue';
import ServicesPage from '../pages/ServicesPage.vue';
import AccountsUsersPage from '../pages/AccountsUsersPage.vue';
import AccountsGroupsPage from '../pages/AccountsGroupsPage.vue';
import PfPage from '../pages/PfPage.vue';
import BhyveVmsPage from '../pages/BhyveVmsPage.vue';
import BhyveCreatePage from '../pages/BhyveCreatePage.vue';
import BhyveDetailPage from '../pages/BhyveDetailPage.vue';
import BhyveConsolePage from '../pages/BhyveConsolePage.vue';
import BhyveVncPage from '../pages/BhyveVncPage.vue';
import BhyveImagesPage from '../pages/BhyveImagesPage.vue';
import BhyveSwitchesPage from '../pages/BhyveSwitchesPage.vue';
import JailsListPage from '../pages/JailsListPage.vue';
import JailCreatePage from '../pages/JailCreatePage.vue';
import JailDetailPage from '../pages/JailDetailPage.vue';
import JailEditPage from '../pages/JailEditPage.vue';
import JailTerminalPage from '../pages/JailTerminalPage.vue';
import JailBasesPage from '../pages/JailBasesPage.vue';
import JailDefaultsPage from '../pages/JailDefaultsPage.vue';
import ZfsPoolsPage from '../pages/ZfsPoolsPage.vue';
import ZfsPoolDetailPage from '../pages/ZfsPoolDetailPage.vue';
import ZfsDatasetsPage from '../pages/ZfsDatasetsPage.vue';
import ZfsSnapshotsPage from '../pages/ZfsSnapshotsPage.vue';
import PackagesPage from '../pages/PackagesPage.vue';
import PackageDetailPage from '../pages/PackageDetailPage.vue';
import PkgReposPage from '../pages/PkgReposPage.vue';
import NotFoundPage from '../pages/NotFoundPage.vue';

const routes = [
  { path: '/login', name: 'login', component: LoginPage, meta: { auth: false } },
  { path: '/setup', name: 'setup', component: SetupPage, meta: { auth: false } },
  {
    path: '/',
    component: AppLayout,
    meta: { auth: true },
    children: [
      { path: '', redirect: '/dashboard' },
      { path: 'dashboard', name: 'dashboard', component: DashboardPage },
      { path: 'shell', name: 'shell', component: ShellPage },
      { path: 'users', name: 'users', component: UsersPage },
      { path: 'audit', name: 'audit', component: AuditPage },
      { path: 'monitor', name: 'monitor-cpu', component: MonitorCpuPage },
      { path: 'monitor/memory', name: 'monitor-memory', component: MonitorMemoryPage },
      { path: 'monitor/network', name: 'monitor-network', component: MonitorNetworkPage },
      { path: 'filesystem', name: 'filesystem', component: FilesystemOverviewPage },
      { path: 'filesystem/disks', name: 'disks', component: DisksPage },
      { path: 'filesystem/files', name: 'files', component: FilesPage },
      { path: 'sysctl', name: 'sysctl', component: SysctlPage },
      { path: 'rcconf', name: 'rcconf', component: RcconfPage },
      { path: 'cron', name: 'cron', component: CronPage },
      { path: 'network', name: 'network', component: NetworkPage },
      { path: 'network/dns', name: 'dns', component: DnsPage },
      { path: 'services', name: 'services', component: ServicesPage },
      { path: 'pf', name: 'pf', component: PfPage },
      { path: 'bhyve', redirect: '/bhyve/vms' },
      { path: 'bhyve/vms', name: 'bhyve-vms', component: BhyveVmsPage },
      { path: 'bhyve/create', name: 'bhyve-create', component: BhyveCreatePage },
      { path: 'bhyve/detail/:name', name: 'bhyve-detail', component: BhyveDetailPage },
      { path: 'bhyve/console/:name', name: 'bhyve-console', component: BhyveConsolePage },
      { path: 'bhyve/vnc/:name', name: 'bhyve-vnc', component: BhyveVncPage },
      { path: 'bhyve/images', name: 'bhyve-images', component: BhyveImagesPage },
      { path: 'bhyve/switches', name: 'bhyve-switches', component: BhyveSwitchesPage },
      { path: 'accounts/users', name: 'accounts-users', component: AccountsUsersPage },
      { path: 'accounts/groups', name: 'accounts-groups', component: AccountsGroupsPage },
      { path: 'jails/running', name: 'jails-list', component: JailsListPage },
      { path: 'jails/create', name: 'jail-create', component: JailCreatePage },
      { path: 'jails/detail/:name', name: 'jail-detail', component: JailDetailPage },
      { path: 'jails/edit/:name', name: 'jail-edit', component: JailEditPage },
      { path: 'jails/terminal/:name', name: 'jail-terminal', component: JailTerminalPage },
      { path: 'jails/bases', name: 'jail-bases', component: JailBasesPage },
      { path: 'jails/defaults', name: 'jail-defaults', component: JailDefaultsPage },
      { path: 'zfs/pools', name: 'zfs-pools', component: ZfsPoolsPage },
      { path: 'zfs/pools/:name', name: 'zfs-pool-detail', component: ZfsPoolDetailPage },
      { path: 'zfs/datasets', name: 'zfs-datasets', component: ZfsDatasetsPage },
      { path: 'zfs/snapshots', name: 'zfs-snapshots', component: ZfsSnapshotsPage },
      { path: 'pkg', name: 'packages', component: PackagesPage },
      { path: 'pkg/repos', name: 'pkg-repos', component: PkgReposPage },
      { path: 'pkg/:name', name: 'package-detail', component: PackageDetailPage },
    ],
  },
  { path: '/:pathMatch(.*)*', name: 'not-found', component: NotFoundPage },
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

  return true;
});

export default router;
