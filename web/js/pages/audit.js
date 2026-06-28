// Audit log page.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';

export async function renderAudit(app) {
  renderLayout(app, '/audit', `
    <div class="page-header">
      <h1>审计日志</h1>
      <p>最近 200 条操作记录</p>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>时间</th><th>用户</th><th>方法</th><th>路径</th><th>状态</th><th>详情</th></tr></thead>
        <tbody id="audit-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> 加载中…</td></tr>
        </tbody>
      </table>
    </div>
  `);

  const tbody = document.getElementById('audit-tbody');
  try {
    const res = await api.get('/api/audit?limit=200');
    if (!res.entries || !res.entries.length) {
      tbody.innerHTML = `<tr><td colspan="6" class="empty">暂无日志</td></tr>`;
      return;
    }
    tbody.innerHTML = res.entries.map(e => `
      <tr>
        <td class="mono text-dim">${fmtTime(e.ts)}</td>
        <td>${esc(e.user || '—')}</td>
        <td><span class="badge ${methodBadge(e.method)}">${esc(e.method)}</span></td>
        <td class="mono">${esc(e.path)}</td>
        <td><span class="badge ${statusBadge(e.status)}">${e.status}</span></td>
        <td class="text-dim">${esc(e.detail || '')}</td>
      </tr>`).join('');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">加载失败：${esc(err.message || '')}</td></tr>`;
  }
}

function fmtTime(ts) {
  return new Date(ts * 1000).toLocaleString('zh-CN');
}
function methodBadge(m) {
  if (m === 'GET') return 'badge-dim';
  if (m === 'DELETE') return 'badge-danger';
  return 'badge-warn';
}
function statusBadge(s) {
  if (s >= 200 && s < 300) return 'badge-success';
  if (s >= 400 && s < 500) return 'badge-warn';
  if (s >= 500) return 'badge-danger';
  return 'badge-dim';
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
