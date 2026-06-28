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
      {
        path: '/accounts/users',
        label: '用户与组',
        icon: '☻',
        children: [
          { path: '/accounts/users', label: '用户', icon: '☻' },
          { path: '/accounts/groups', label: '用户组', icon: '☰' },
        ],
      },
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
      { path: '/filesystem/files', label: '文件管理器', icon: '📁' },
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
];

// Settings menu (right-side dropdown) — panel self-management.
const SETTINGS = [
  { path: '/users', label: '用户', icon: '☻' },
  { path: '/audit', label: '审计日志', icon: '☰' },
];

// Determine which top-level group a path belongs to.
function groupOfPath(path) {
  for (const g of MENU) {
    if (pathBelongsToGroup(path, g.items)) return g.key;
  }
  if (pathBelongsToGroup(path, SETTINGS)) return 'settings';
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

// Close the settings dropdown when clicking outside it.
// Bound once on the document; re-binding on each render is a no-op.
let settingsDocClickBound = false;
function bindSettingsDocClick() {
  if (settingsDocClickBound) return;
  document.addEventListener('click', (e) => {
    const settingsMenu = document.getElementById('settings-menu');
    if (settingsMenu && !settingsMenu.contains(e.target)) settingsMenu.classList.remove('open');
    const userMenu = document.getElementById('user-menu');
    if (userMenu && !userMenu.contains(e.target)) {
      userMenu.classList.remove('open');
      const toggle = document.getElementById('nav-user');
      if (toggle) toggle.setAttribute('aria-expanded', 'false');
    }
  });
  settingsDocClickBound = true;
}

export function renderLayout(app, currentPath, pageContent) {
  const activeGroup = groupOfPath(currentPath);
  const group = activeGroup === 'settings'
    ? { items: SETTINGS }
    : (MENU.find((g) => g.key === activeGroup) || MENU[0]);

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
        <div class="settings-menu" id="settings-menu">
          <button class="settings-btn ${activeGroup === 'settings' ? 'active' : ''}" id="settings-toggle" aria-haspopup="true" aria-expanded="false">
            <span class="icon">⚙</span>设置
          </button>
          <div class="settings-dropdown">
            ${SETTINGS.map((s) => `
              <a href="#${s.path}" class="${currentPath === s.path ? 'active' : ''}">
                <span class="icon">${s.icon}</span>${s.label}
              </a>`).join('')}
          </div>
        </div>
        <div class="settings-menu" id="user-menu">
          <button class="user-chip" id="nav-user" aria-haspopup="true" aria-expanded="false">
            <span class="icon">👤</span><span class="user-name">…</span>
          </button>
          <div class="settings-dropdown">
            <a href="#" id="nav-logout">
              <span class="icon">⏻</span>退出登录
            </a>
          </div>
        </div>
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

  // Settings dropdown: toggle on button click, close on item selection.
  const settingsToggle = document.getElementById('settings-toggle');
  if (settingsToggle) {
    settingsToggle.addEventListener('click', (e) => {
      e.stopPropagation();
      const menu = document.getElementById('settings-menu');
      const open = menu.classList.toggle('open');
      settingsToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
    });
  }
  const settingsDropdown = document.querySelector('.settings-dropdown');
  if (settingsDropdown) {
    settingsDropdown.addEventListener('click', () => {
      const menu = document.getElementById('settings-menu');
      menu.classList.remove('open');
      const toggle = document.getElementById('settings-toggle');
      if (toggle) toggle.setAttribute('aria-expanded', 'false');
    });
  }
  bindSettingsDocClick();

  // User dropdown: toggle on chip click, logout on item click.
  const userToggle = document.getElementById('nav-user');
  if (userToggle) {
    userToggle.addEventListener('click', (e) => {
      e.stopPropagation();
      const menu = document.getElementById('user-menu');
      const open = menu.classList.toggle('open');
      userToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
    });
  }
  const navLogout = document.getElementById('nav-logout');
  if (navLogout) {
    navLogout.addEventListener('click', (e) => {
      e.preventDefault();
      const menu = document.getElementById('user-menu');
      menu.classList.remove('open');
      window.__fwpLogout();
    });
  }
  import('../api.js').then(({ api }) => {
    api.get('/api/auth/me').then((u) => {
      const el = document.querySelector('.user-name');
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
