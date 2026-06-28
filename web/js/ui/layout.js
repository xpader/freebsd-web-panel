// Application shell: topbar (primary) + sidebar (secondary + tertiary).
//
// Menu item shape:
//   { path, label, icon, children: [{ path, label, icon }] }
// If `children` exists, the item is a collapsible group (no direct route);
// otherwise it's a direct link.

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
      { path: '/services', label: '服务', icon: '▶' },
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
    default: '/filesystem',
    items: [
      { path: '/filesystem', label: '概览', icon: '◇' },
      { path: '/filesystem/disks', label: '磁盘', icon: '▤' },
      {
        path: '/zfs',
        label: 'ZFS',
        icon: '◈',
        children: [
          { path: '/zfs/pools', label: 'Zpool', icon: '◉' },
          { path: '/zfs/datasets', label: '数据集', icon: '◇' },
          { path: '/zfs/snapshots', label: '快照', icon: '⎙' },
        ],
      },
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
    key: 'monitor',
    label: '监控',
    icon: '📊',
    default: '/monitor',
    items: [
      { path: '/monitor', label: 'CPU & 负载', icon: '📊' },
      { path: '/monitor/memory', label: '内存', icon: '▦' },
      { path: '/monitor/temp', label: '温度', icon: '🌡' },
    ],
  },
  {
    key: 'system',
    label: '系统',
    icon: '☻',
    default: '/users',
    items: [
      { path: '/users', label: '用户', icon: '☻' },
      { path: '/audit', label: '审计日志', icon: '☰' },
    ],
  },
];

// Determine which top-level group a path belongs to.
function groupOfPath(path) {
  for (const g of MENU) {
    if (pathBelongsToGroup(path, g.items)) return g.key;
  }
  return 'overview';
}

function pathBelongsToGroup(path, items) {
  for (const item of items) {
    if (path === item.path) return true;
    if (item.children) {
      for (const child of item.children) {
        if (path === child.path) return true;
      }
    }
  }
  return false;
}

export function renderLayout(app, currentPath, pageContent) {
  const activeGroup = groupOfPath(currentPath);
  const group = MENU.find((g) => g.key === activeGroup) || MENU[0];

  // Top menu tabs.
  const topHtml = MENU.map((g) => `
    <a href="#${g.default}" class="topnav-tab ${g.key === activeGroup ? 'active' : ''}">
      <span class="icon">${g.icon}</span>${g.label}
    </a>`).join('');

  // Sidebar items — supports expandable children.
  const subHtml = group.items
    .map((item) => renderSidebarItem(item, currentPath))
    .join('');

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

  // Attach click handlers for collapsible sub-groups.
  // Clicking a group header navigates to its first child route,
  // which also auto-expands the group via hasActiveChild.
  app.querySelectorAll('.sub-group-header').forEach((header) => {
    header.addEventListener('click', () => {
      const firstChild = header.parentElement.querySelector('.sub-item');
      if (firstChild) {
        const href = firstChild.getAttribute('href');
        if (href) location.hash = href;
      }
    });
  });

  import('../api.js').then(({ api }) => {
    api.get('/api/auth/me').then((u) => {
      const el = document.getElementById('nav-user');
      if (el) el.textContent = u.username;
    }).catch(() => {});
  });
}

function renderSidebarItem(item, currentPath) {
  // Item with children — collapsible group.
  if (item.children) {
    // Determine if this group is expanded (any child is active).
    const hasActiveChild = item.children.some((c) => currentPath === c.path);
    const childHtml = item.children
      .map(
        (c) => `
        <a href="#${c.path}" class="sub-item ${currentPath === c.path ? 'active' : ''}">
          <span class="icon">${c.icon}</span>${c.label}
        </a>`,
      )
      .join('');
    return `
      <div class="sub-group ${hasActiveChild ? 'expanded' : ''}">
        <div class="sub-group-header">
          <span class="icon">${item.icon}</span>${item.label}
          <span class="sub-arrow">${hasActiveChild ? '▾' : '▸'}</span>
        </div>
        <div class="sub-items">${childHtml}</div>
      </div>`;
  }

  // Direct link item.
  return `
    <a href="#${item.path}" class="${currentPath === item.path ? 'active' : ''}">
      <span class="icon">${item.icon}</span>${item.label}
    </a>`;
}
