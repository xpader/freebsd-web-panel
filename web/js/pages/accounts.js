// System accounts — FreeBSD users & groups list pages.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';

export async function renderSysUsers(app) {
  renderLayout(app, '/accounts/users', `
    <div class="page-header">
      <h1>用户</h1>
      <p>FreeBSD 系统用户（来自 /etc/passwd）</p>
    </div>
    <div class="toolbar">
      <input type="text" id="user-filter" class="filter-input" placeholder="筛选用户名 / UID…" />
      <span id="user-count" class="text-dim"></span>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>用户名</th><th>UID</th><th>主组</th><th>描述</th><th>家目录</th><th>Shell</th></tr></thead>
        <tbody id="sysusers-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> 加载中…</td></tr>
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
    tbody.innerHTML = `<tr><td colspan="6" class="empty">加载失败：${esc(err.message || '')}</td></tr>`;
    return;
  }

  function render() {
    const q = filter.value.trim().toLowerCase();
    const list = q
      ? allUsers.filter((u) =>
          u.name.toLowerCase().includes(q) || String(u.uid).includes(q))
      : allUsers;
    countEl.textContent = `共 ${list.length} 个用户`;
    if (!list.length) {
      tbody.innerHTML = `<tr><td colspan="6" class="empty">无匹配用户</td></tr>`;
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
      <h1>用户组</h1>
      <p>FreeBSD 系统用户组（来自 /etc/group）</p>
    </div>
    <div class="toolbar">
      <input type="text" id="group-filter" class="filter-input" placeholder="筛选组名 / GID…" />
      <span id="group-count" class="text-dim"></span>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>组名</th><th>GID</th><th>成员</th></tr></thead>
        <tbody id="sysgroups-tbody">
          <tr><td colspan="3" class="empty"><span class="spinner"></span> 加载中…</td></tr>
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
    tbody.innerHTML = `<tr><td colspan="3" class="empty">加载失败：${esc(err.message || '')}</td></tr>`;
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
    countEl.textContent = `共 ${list.length} 个用户组`;
    if (!list.length) {
      tbody.innerHTML = `<tr><td colspan="3" class="empty">无匹配用户组</td></tr>`;
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
