// Application shell: topbar (primary) + sidebar (secondary + tertiary).
//
// Menu item shape:
//   { path, labelKey, icon, default, items: [{ path, labelKey, icon, children }] }
// `labelKey` is an i18n key resolved at render time. If `children` exists,
// the item is a collapsible group (no direct route); otherwise it's a direct link.

import { t, LANGUAGES, getLang, setLang, currentLangMeta } from '../i18n/index.js';

const MENU = [
  {
    key: 'overview',
    labelKey: 'nav.overview',
    icon: 'fa-solid fa-gauge-high',
    default: '/dashboard',
    items: [
      { path: '/dashboard', labelKey: 'nav.dashboard', icon: 'fa-solid fa-gauge-high' },
    ],
  },
  {
    key: 'config',
    labelKey: 'nav.config',
    icon: 'fa-solid fa-sliders',
    default: '/sysctl',
    items: [
      { path: '/sysctl', labelKey: 'nav.sysctl', icon: 'fa-solid fa-microchip' },
      { path: '/rcconf', labelKey: 'nav.rcconf', icon: 'fa-solid fa-list-check' },
      { path: '/services', labelKey: 'nav.services', icon: 'fa-solid fa-play' },
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
    key: 'network',
    labelKey: 'nav.network',
    icon: 'fa-solid fa-network-wired',
    default: '/network',
    items: [
      { path: '/network', labelKey: 'nav.networkIf', icon: 'fa-solid fa-ethernet' },
      { path: '/pf', labelKey: 'nav.pf', icon: 'fa-solid fa-shield-halved' },
    ],
  },
  {
    key: 'filesystem',
    labelKey: 'nav.filesystem',
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
    default: '/jails',
    items: [
      { path: '/jails', labelKey: 'nav.jails', icon: 'fa-solid fa-cube' },
      { path: '/bhyve', labelKey: 'nav.bhyve', icon: 'fa-regular fa-square' },
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
      { path: '/monitor/temp', labelKey: 'nav.monitorTemp', icon: 'fa-solid fa-temperature-half' },
    ],
  },
];

// Settings menu (right-side dropdown) — panel self-management.
const SETTINGS = [
  { path: '/users', labelKey: 'topbar.panelUsers', icon: 'fa-solid fa-user-gear' },
  { path: '/audit', labelKey: 'topbar.auditLog', icon: 'fa-solid fa-list-ul' },
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

// Close the settings/language dropdowns when clicking outside them.
// Bound once on the document; re-binding on each render is a no-op.
let settingsDocClickBound = false;
function bindSettingsDocClick() {
  if (settingsDocClickBound) return;
  document.addEventListener('click', (e) => {
    const settingsMenu = document.getElementById('settings-menu');
    if (settingsMenu && !settingsMenu.contains(e.target)) settingsMenu.classList.remove('open');
    const langMenu = document.getElementById('lang-menu');
    if (langMenu && !langMenu.contains(e.target)) langMenu.classList.remove('open');
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
      <span class="icon"><i class="${g.icon}"></i></span>${t(g.labelKey)}
    </a>`).join('');

  // Sidebar items — supports expandable children.
  const subHtml = group.items
    .map((item) => renderSidebarItem(item, currentPath))
    .join('');

  // Language switcher: current flag as the trigger, full list in the dropdown.
  const curLang = currentLangMeta();
  const langItems = LANGUAGES.map((l) => `
    <a href="#" class="lang-item ${l.code === curLang.code ? 'active' : ''}" data-lang="${l.code}">
      <span class="icon lang-flag">${l.flag}</span>${l.label}
    </a>`).join('');

  app.innerHTML = `
    <div class="topbar">
      <div class="topbar-brand">FreeBSD Web Panel</div>
      <nav class="topnav">${topHtml}</nav>
      <div class="topbar-right">
        <div class="settings-menu" id="lang-menu">
          <button class="lang-btn" id="lang-toggle" aria-haspopup="true" aria-expanded="false" title="${t('topbar.language')}">
            <span class="icon lang-flag">${curLang.flag}</span>
          </button>
          <div class="settings-dropdown">
            ${langItems}
          </div>
        </div>
        <div class="settings-menu" id="settings-menu">
          <button class="settings-btn ${activeGroup === 'settings' ? 'active' : ''}" id="settings-toggle" aria-haspopup="true" aria-expanded="false">
            <span class="icon"><i class="fa-solid fa-gear"></i></span>${t('topbar.settings')}
          </button>
          <div class="settings-dropdown">
            ${SETTINGS.map((s) => `
              <a href="#${s.path}" class="${currentPath === s.path ? 'active' : ''}">
                <span class="icon"><i class="${s.icon}"></i></span>${t(s.labelKey)}
              </a>`).join('')}
          </div>
        </div>
        <div class="settings-menu" id="user-menu">
          <button class="user-chip" id="nav-user" aria-haspopup="true" aria-expanded="false">
            <span class="icon"><i class="fa-solid fa-circle-user"></i></span><span class="user-name">…</span>
          </button>
          <div class="settings-dropdown">
            <a href="#" id="nav-logout">
              <span class="icon"><i class="fa-solid fa-power-off"></i></span>${t('topbar.logout')}
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

  // Language dropdown: toggle on button click, switch on item click.
  const langToggle = document.getElementById('lang-toggle');
  if (langToggle) {
    langToggle.addEventListener('click', (e) => {
      e.stopPropagation();
      document.getElementById('lang-menu').classList.toggle('open');
    });
  }
  const langMenu = document.getElementById('lang-menu');
  if (langMenu) {
    langMenu.querySelectorAll('.lang-item').forEach((item) => {
      item.addEventListener('click', (e) => {
        e.preventDefault();
        const code = item.getAttribute('data-lang');
        langMenu.classList.remove('open');
        setLang(code);
      });
    });
  }

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
  const settingsDropdown = document.querySelector('#settings-menu .settings-dropdown');
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
          <span class="icon"><i class="${c.icon}"></i></span>${t(c.labelKey)}
        </a>`,
      )
      .join('');
    return `
      <div class="sub-group ${hasActiveChild ? 'expanded' : ''}">
        <div class="sub-group-header">
          <span class="icon"><i class="${item.icon}"></i></span>${t(item.labelKey)}
          <span class="sub-arrow">${hasActiveChild ? '<i class="fa-solid fa-caret-down"></i>' : '<i class="fa-solid fa-caret-right"></i>'}</span>
        </div>
        <div class="sub-items">${childHtml}</div>
      </div>`;
  }

  // Direct link item.
  return `
    <a href="#${item.path}" class="${currentPath === item.path ? 'active' : ''}">
      <span class="icon"><i class="${item.icon}"></i></span>${t(item.labelKey)}
    </a>`;
}
