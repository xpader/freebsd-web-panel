// System accounts — FreeBSD users & groups list pages.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

export async function renderSysUsers(app) {
  renderLayout(app, '/accounts/users', `
    <div class="page-header">
      <h1>${t('accounts.usersTitle')}</h1>
      <p>${t('accounts.usersSubtitle')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="user-filter" class="filter-input" placeholder="${t('accounts.filterUser')}" />
      <span id="user-count" class="text-dim"></span>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('auth.username')}</th><th>${t('accounts.uid')}</th><th>${t('accounts.group')}</th><th>${t('common.description')}</th><th>${t('accounts.home')}</th><th>Shell</th></tr></thead>
        <tbody id="sysusers-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);

  let allUsers = [];
  const tbody = document.getElementById('sysusers-tbody');
  const filter = document.getElementById('user-filter');
  const countEl = document.getElementById('user-count');

  try {
    allUsers = await api.get('/api/accounts/users');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }

  function render() {
    const q = filter.value.trim().toLowerCase();
    const list = q
      ? allUsers.filter((u) =>
          u.name.toLowerCase().includes(q) || String(u.uid).includes(q))
      : allUsers;
    countEl.textContent = t('accounts.userCount', { n: list.length });
    if (!list.length) {
      tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('accounts.noMatchUser')}</td></tr>`;
      return;
    }
    tbody.innerHTML = list.map((u) => `
      <tr>
        <td><strong>${esc(u.name)}</strong></td>
        <td class="mono">${u.uid}</td>
        <td class="mono">${esc(u.group_name || '—')} <span class="text-dim">(${u.gid})</span></td>
        <td class="text-dim">${esc(u.gecos) || '—'}</td>
        <td class="mono">${esc(u.home)}</td>
        <td class="mono">${esc(u.shell)}</td>
      </tr>`).join('');
  }

  filter.addEventListener('input', render);
  render();
}

export async function renderSysGroups(app) {
  renderLayout(app, '/accounts/groups', `
    <div class="page-header">
      <h1>${t('accounts.groupsTitle')}</h1>
      <p>${t('accounts.groupsSubtitle')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="group-filter" class="filter-input" placeholder="${t('accounts.filterGroup')}" />
      <span id="group-count" class="text-dim"></span>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('auth.username')}</th><th>${t('accounts.gid')}</th><th>${t('accounts.members')}</th></tr></thead>
        <tbody id="sysgroups-tbody">
          <tr><td colspan="3" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);

  let allGroups = [];
  const tbody = document.getElementById('sysgroups-tbody');
  const filter = document.getElementById('group-filter');
  const countEl = document.getElementById('group-count');

  try {
    allGroups = await api.get('/api/accounts/groups');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="3" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }

  function render() {
    const q = filter.value.trim().toLowerCase();
    const list = q
      ? allGroups.filter((g) =>
          g.name.toLowerCase().includes(q) ||
          String(g.gid).includes(q) ||
          g.members.some((m) => m.toLowerCase().includes(q)))
      : allGroups;
    countEl.textContent = t('accounts.groupCount', { n: list.length });
    if (!list.length) {
      tbody.innerHTML = `<tr><td colspan="3" class="empty">${t('accounts.noMatchGroup')}</td></tr>`;
      return;
    }
    tbody.innerHTML = list.map((g) => `
      <tr>
        <td><strong>${esc(g.name)}</strong></td>
        <td class="mono">${g.gid}</td>
        <td>${g.members.length
          ? g.members.map((m) => `<span class="badge badge-dim">${esc(m)}</span>`).join(' ')
          : '<span class="text-dim">—</span>'}</td>
      </tr>`).join('');
  }

  filter.addEventListener('input', render);
  render();
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
