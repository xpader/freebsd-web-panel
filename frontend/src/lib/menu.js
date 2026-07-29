// Navigation menu configuration — top-level groups + sidebar items.

export const MENU = [
  {
    key: 'overview',
    labelKey: 'nav.overview',
    icon: 'fa-solid fa-gauge-high',
    default: '/dashboard',
    items: [
      { path: '/dashboard', labelKey: 'nav.dashboard', icon: 'fa-solid fa-gauge-high' },
      { path: '/shell', labelKey: 'nav.shell', icon: 'fa-solid fa-terminal' },
    ],
  },
  {
    key: 'system',
    labelKey: 'nav.system',
    icon: 'fa-solid fa-sliders',
    default: '/rcconf',
    items: [
      { path: '/rcconf', labelKey: 'nav.rcconf', icon: 'fa-solid fa-list-check' },
      { path: '/sysctl', labelKey: 'nav.sysctl', icon: 'fa-solid fa-microchip' },
      { path: '/cron', labelKey: 'nav.cron', icon: 'fa-solid fa-clock-rotate-left' },
      {
        path: '/pkg',
        labelKey: 'nav.packages',
        icon: 'fa-solid fa-box',
        children: [
          { path: '/pkg', labelKey: 'nav.pkgList', icon: 'fa-solid fa-list' },
          { path: '/pkg/repos', labelKey: 'nav.pkgRepos', icon: 'fa-solid fa-server' },
        ],
      },
      {
        path: '/accounts/users',
        labelKey: 'nav.accounts',
        icon: 'fa-solid fa-users',
        children: [
          { path: '/accounts/users', labelKey: 'nav.sysUsers', icon: 'fa-solid fa-user' },
          { path: '/accounts/groups', labelKey: 'nav.sysGroups', icon: 'fa-solid fa-users-rectangle' },
        ],
      },
    ],
  },
  {
    key: 'services',
    labelKey: 'nav.services',
    icon: 'fa-solid fa-atom',
    default: '/services',
    items: [
      { path: '/services', labelKey: 'nav.systemServices', icon: 'fa-solid fa-play' },
      {
        path: '/shares/smb',
        labelKey: 'nav.smb',
        icon: 'fa-solid fa-share-nodes',
        children: [
          { path: '/shares/smb', labelKey: 'nav.smbShares', icon: 'fa-solid fa-folder-tree' },
          { path: '/shares/smb/users', labelKey: 'nav.smbUsers', icon: 'fa-solid fa-user-lock' },
          { path: '/shares/smb/settings', labelKey: 'nav.smbSettings', icon: 'fa-solid fa-gear' },
        ],
      },
    ],
  },
  {
    key: 'network',
    labelKey: 'common.network',
    icon: 'fa-solid fa-network-wired',
    default: '/network',
    items: [
      { path: '/network', labelKey: 'nav.networkIf', icon: 'fa-solid fa-ethernet' },
      { path: '/network/routes', labelKey: 'nav.staticRoutes', icon: 'fa-solid fa-route' },
      { path: '/network/dns', labelKey: 'nav.networkDns', icon: 'fa-solid fa-server' },
      {
        path: '/firewall',
        labelKey: 'nav.pf',
        icon: 'fa-solid fa-shield-halved',
        children: [
          { path: '/firewall/rules', labelKey: 'nav.firewallRules', icon: 'fa-solid fa-list' },
          { path: '/firewall/nat', labelKey: 'nav.firewallNat', icon: 'fa-solid fa-arrow-right-arrow-left' },
          { path: '/firewall/tables', labelKey: 'nav.firewallTables', icon: 'fa-solid fa-table-list' },
          { path: '/firewall/settings', labelKey: 'nav.firewallSettings', icon: 'fa-solid fa-gear' },
        ],
      },
    ],
  },
  {
    key: 'storage',
    labelKey: 'nav.storage',
    icon: 'fa-solid fa-hard-drive',
    default: '/filesystem',
    items: [
      { path: '/filesystem', labelKey: 'nav.fsOverview', icon: 'fa-solid fa-chart-pie' },
      { path: '/filesystem/disks', labelKey: 'nav.disks', icon: 'fa-solid fa-hard-drive' },
      { path: '/filesystem/files', labelKey: 'nav.fileManager', icon: 'fa-solid fa-folder-open' },
      {
        path: '/zfs',
        labelKey: 'nav.zfs',
        icon: 'fa-solid fa-database',
        children: [
          { path: '/zfs/pools', labelKey: 'nav.zpool', icon: 'fa-solid fa-circle-nodes' },
          { path: '/zfs/datasets', labelKey: 'nav.datasets', icon: 'fa-solid fa-layer-group' },
          { path: '/zfs/snapshots', labelKey: 'nav.snapshots', icon: 'fa-solid fa-camera' },
        ],
      },
    ],
  },
  {
    key: 'virtualization',
    labelKey: 'nav.virtualization',
    icon: 'fa-solid fa-cubes',
    default: '/jails/running',
    items: [
      {
        path: '/jails',
        labelKey: 'nav.jails',
        icon: 'fa-solid fa-cube',
        children: [
          { path: '/jails/running', labelKey: 'nav.jailList', icon: 'fa-solid fa-list' },
          { path: '/jails/bases', labelKey: 'nav.jailBases', icon: 'fa-solid fa-layer-group' },
          { path: '/jails/defaults', labelKey: 'nav.jailDefaults', icon: 'fa-solid fa-gears' },
        ],
      },
      {
        path: '/bhyve',
        labelKey: 'nav.bhyve',
        icon: 'fa-solid fa-server',
        children: [
          { path: '/bhyve/vms', labelKey: 'nav.bhyveVms', icon: 'fa-solid fa-list' },
          { path: '/bhyve/switches', labelKey: 'nav.bhyveSwitches', icon: 'fa-solid fa-network-wired' },
          { path: '/bhyve/datastores', labelKey: 'nav.bhyveDatastores', icon: 'fa-solid fa-hard-drive' },
          { path: '/bhyve/images', labelKey: 'nav.bhyveImages', icon: 'fa-solid fa-copy' },
          { path: '/bhyve/isos', labelKey: 'nav.bhyveIsos', icon: 'fa-solid fa-compact-disc' },
        ],
      },
    ],
  },
  {
    key: 'monitor',
    labelKey: 'nav.monitor',
    icon: 'fa-solid fa-chart-line',
    default: '/monitor',
    items: [
      { path: '/monitor', labelKey: 'nav.monitorCpu', icon: 'fa-solid fa-chart-line' },
      { path: '/monitor/memory', labelKey: 'nav.monitorMemory', icon: 'fa-solid fa-memory' },
      { path: '/monitor/network', labelKey: 'common.network', icon: 'fa-solid fa-network-wired' },
      { path: '/mail', labelKey: 'nav.mail', icon: 'fa-solid fa-envelope' },
      { path: '/monitor/fwp', labelKey: 'nav.monitorFwp', icon: 'fa-solid fa-circle-info' },
    ],
  },
];

export const SETTINGS = [
  { path: '/users', labelKey: 'topbar.panelUsers', icon: 'fa-solid fa-user-gear' },
  { path: '/audit', labelKey: 'topbar.auditLog', icon: 'fa-solid fa-list-ul' },
  ...(import.meta.env.DEV ? [{ path: '/dialog-demo', label: '对话框演示', icon: 'fa-solid fa-window-restore' }] : []),
];

// Determine which top-level group a path belongs to.
export function groupOfPath(path) {
  for (const g of MENU) {
    if (pathBelongsToGroup(path, g.items)) return g.key;
  }
  if (pathBelongsToGroup(path, SETTINGS)) return 'settings';
  return 'overview';
}

function pathBelongsToGroup(path, items) {
  for (const item of items) {
    if (path === item.path || path.startsWith(item.path + '/')) return true;
    if (item.children) {
      for (const child of item.children) {
        if (path === child.path || path.startsWith(child.path + '/')) return true;
      }
    }
  }
  return false;
}
