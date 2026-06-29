// User management page.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { t, getLocale } from '../i18n/index.js';

export async function renderUsers(app) {
  renderLayout(app, '/users', `
    <div class="page-header">
      <h1>${t('users.title')}</h1>
      <p>${t('users.subtitle')}</p>
    </div>
    <div class="toolbar">
      <div></div>
      <button onclick="window.__fwpAddUser()">${t('users.add')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>ID</th><th>${t('auth.username')}</th><th>${t('users.colRole')}</th><th>${t('common.colCreatedAt')}</th><th>${t('users.colLastLogin')}</th><th>${t('common.actions')}</th></tr></thead>
        <tbody id="users-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);

  await loadUsers();
}

async function loadUsers() {
  const tbody = document.getElementById('users-tbody');
  try {
    const users = await api.get('/api/users');
    if (!users.length) {
      tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('users.noUsers')}</td></tr>`;
      return;
    }
    tbody.innerHTML = users.map(u => `
      <tr>
        <td class="mono">${u.id}</td>
        <td><strong>${esc(u.username)}</strong></td>
        <td><span class="badge badge-success">${esc(u.role)}</span></td>
        <td class="text-dim mono">${fmtTime(u.created_at)}</td>
        <td class="text-dim mono">${u.last_login ? fmtTime(u.last_login) : '—'}</td>
        <td>
          <button class="btn-secondary btn-sm" onclick="window.__fwpEditPwd(${u.id}, '${esc(u.username)}')">${t('users.changePwd')}</button>
          <button class="btn-danger btn-sm" onclick="window.__fwpDelUser(${u.id}, '${esc(u.username)}')">${t('common.delete')}</button>
        </td>
      </tr>`).join('');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
  }
}

window.__fwpAddUser = () => {
  showModal(t('users.addUser'), '', '', async (username, password) => {
    await api.post('/api/users', { username, password });
    toast(t('users.created'));
    loadUsers();
  });
};

window.__fwpEditPwd = (id, name) => {
  showModal(t('users.editPwdTitle', { name }), '', '', async (_u, password) => {
    await api.put(`/api/users/${id}`, { password });
    toast(t('users.pwdUpdated'));
  });
};

window.__fwpDelUser = async (id, name) => {
  if (!await confirmDialog(t('users.deleteUser'), t('users.deleteConfirm', { name }))) return;
  try {
    await api.del(`/api/users/${id}`);
    toast(t('users.deleted'));
    loadUsers();
  } catch (err) {
    toast(err.message || t('users.deleteFailed'), 'error');
  }
};

function showModal(title, defaultName, defaultPwd, onSubmit) {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal">
      <h3>${title}</h3>
      <form id="modal-form">
        <div class="field">
          <label>${t('auth.username')}</label>
          <input type="text" name="username" value="${esc(defaultName)}" required />
        </div>
        <div class="field">
          <label>${t('auth.passwordMin')}</label>
          <input type="password" name="password" value="${esc(defaultPwd)}" required minlength="6" />
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
          <button type="submit">${t('common.ok')}</button>
        </div>
      </form>
    </div>`;
  document.body.appendChild(overlay);

  overlay.addEventListener('click', (e) => {
    if (e.target === overlay || e.target.dataset.act === 'cancel') overlay.remove();
  });
  overlay.querySelector('#modal-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const form = e.target;
    try {
      await onSubmit(form.username.value, form.password.value);
      overlay.remove();
    } catch (err) {
      toast(err.message || t('common.operationFailed'), 'error');
    }
  });
}

function fmtTime(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleString(getLocale());
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
