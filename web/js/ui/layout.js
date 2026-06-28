// Application shell: topbar (primary menu) + sidebar (secondary menu).

// Menu structure: top-level groups, each with its secondary items.
// `default` is the route the top tab navigates to when clicked.
const MENU = [
  {
    key: 'overview',
    label: '概览',
    icon: '◎',
    default: '/dashboard',
    items: [
      { path: '/dashboard', label: '仪表盘', icon: '◎' },
    ],
  },
  {
    key: 'config',
    label: '配置',
    icon: '⚙',
    default: '/sysctl',
    items: [
      { path: '/sysctl', label: 'Sysctl 系统参数', icon: '⚙' },
      { path: '/rcconf', label: 'RC 配置', icon: '☰' },
      { path: '/services', label: '服务管理', icon: '▶' },
    ],
  },
  {
    key: 'network',
    label: '网络',
    icon: '⇄',
    default: '/network',
    items: [
      { path: '/network', label: '网络接口', icon: '⇄' },
      { path: '/pf', label: '防火墙 (PF)', icon: '🛡' },
    ],
  },
  {
    key: 'filesystem',
    label: '文件系统',
    icon: '◈',
    default: '/zfs',
    items: [
      { path: '/zfs', label: 'ZFS', icon: '◈' },
    ],
  },
  {
    key: 'virtualization',
    label: '虚拟化',
    icon: '▣',
    default: '/jails',
    items: [
      { path: '/jails', label: 'Jail 容器', icon: '▣' },
      { path: '/bhyve', label: 'Bhyve 虚拟机', icon: '▢' },
    ],
  },
  {
    key: 'system',
    label: '系统',
    icon: '☻',
    default: '/users',
    items: [
      { path: '/users', label: '用户管理', icon: '☻' },
      { path: '/audit', label: '审计日志', icon: '☰' },
    ],
  },
];

// Flatten all paths → which top group they belong to.
function groupOfPath(path) {
  for (const g of MENU) {
    if (g.items.some(i => path === i.path || path.startsWith(i.path + '/'))) {
      return g.key;
    }
  }
  return 'overview';
}

export function renderLayout(app, currentPath, pageContent) {
  const activeGroup = groupOfPath(currentPath);
  const group = MENU.find(g => g.key === activeGroup) || MENU[0];

  // Top menu tabs.
  const topHtml = MENU.map(g => `
    <a href="#${g.default}" class="topnav-tab ${g.key === activeGroup ? 'active' : ''}">
      <span class="icon">${g.icon}</span>${g.label}
    </a>`).join('');

  // Sidebar items for the active group.
  const subHtml = group.items.map(item => `
    <a href="#${item.path}" class="${currentPath === item.path || currentPath.startsWith(item.path + '/') ? 'active' : ''}">
      <span class="icon">${item.icon}</span>${item.label}
    </a>`).join('');

  app.innerHTML = `
    <div class="topbar">
      <div class="topbar-brand">FreeBSD Web Panel</div>
      <nav class="topnav">${topHtml}</nav>
      <div class="topbar-right">
        <span class="user-chip" id="nav-user">…</span>
        <button class="btn-secondary btn-sm" onclick="window.__fwpLogout()">退出</button>
      </div>
    </div>
    <div class="body-wrap">
      <aside class="sidebar">
        <nav class="sidebar-nav">${subHtml}</nav>
      </aside>
      <main class="main">${pageContent}</main>
    </div>
  `;

  // Load current user info into topbar.
  import('../api.js').then(({ api }) => {
    api.get('/api/auth/me').then(u => {
      const el = document.getElementById('nav-user');
      if (el) el.textContent = u.username;
    }).catch(() => {});
  });
}
