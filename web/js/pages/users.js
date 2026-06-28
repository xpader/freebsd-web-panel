// User management page.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';

export async function renderUsers(app) {
  renderLayout(app, '/users', `
    <div class="page-header">
      <h1>用户</h1>
      <p>管理面板管理员账户</p>
    </div>
    <div class="toolbar">
      <div></div>
      <button onclick="window.__fwpAddUser()">+ 添加用户</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>ID</th><th>用户名</th><th>角色</th><th>创建时间</th><th>最后登录</th><th>操作</th></tr></thead>
        <tbody id="users-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> 加载中…</td></tr>
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
      tbody.innerHTML = `<tr><td colspan="6" class="empty">暂无用户</td></tr>`;
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
          <button class="btn-secondary btn-sm" onclick="window.__fwpEditPwd(${u.id}, '${esc(u.username)}')">改密</button>
          <button class="btn-danger btn-sm" onclick="window.__fwpDelUser(${u.id}, '${esc(u.username)}')">删除</button>
        </td>
      </tr>`).join('');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">加载失败：${esc(err.message || '')}</td></tr>`;
  }
}

window.__fwpAddUser = () => {
  showModal('添加用户', '', '', async (username, password) => {
    await api.post('/api/users', { username, password });
    toast('用户创建成功');
    loadUsers();
  });
};

window.__fwpEditPwd = (id, name) => {
  showModal(`修改密码：${name}`, '', '', async (_u, password) => {
    await api.put(`/api/users/${id}`, { password });
    toast('密码已更新');
  });
};

window.__fwpDelUser = async (id, name) => {
  if (!await confirmDialog('删除用户', `确定删除用户 "${name}" 吗？此操作不可撤销。`)) return;
  try {
    await api.del(`/api/users/${id}`);
    toast('用户已删除');
    loadUsers();
  } catch (err) {
    toast(err.message || '删除失败', 'error');
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
          <label>用户名</label>
          <input type="text" name="username" value="${esc(defaultName)}" required />
        </div>
        <div class="field">
          <label>密码（至少 6 位）</label>
          <input type="password" name="password" value="${esc(defaultPwd)}" required minlength="6" />
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" data-act="cancel">取消</button>
          <button type="submit">确定</button>
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
      toast(err.message || '操作失败', 'error');
    }
  });
}

function fmtTime(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleString('zh-CN');
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
