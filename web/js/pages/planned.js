// Generic "planned" placeholder page for FreeBSD management modules.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';

export function makePlannedPage(path, label, description, detail) {
  return async function (app) {
    renderLayout(app, path, `
      <div class="page-header">
        <h1>${label}</h1>
        <p>${description}</p>
      </div>
      <div class="card" id="module-status-card">
        <div class="empty"><span class="spinner"></span> 检查模块状态…</div>
      </div>
      <div class="card">
        <div class="card-title">实现计划</div>
        <p class="text-dim">${detail}</p>
        <p class="text-dim mt-8">该模块将在框架确认后，于后续阶段实施。当前为骨架占位。</p>
      </div>
    `);

    const card = document.getElementById('module-status-card');
    try {
      const status = await api.get(`/api${path}`);
      card.innerHTML = `
        <div class="card-title">模块状态</div>
        <div class="flex">
          <span class="badge ${status.status === 'planned' ? 'badge-warn' : 'badge-success'}">${esc(status.status)}</span>
          <span class="text-dim">${esc(status.message)}</span>
        </div>`;
    } catch (err) {
      card.innerHTML = `<div class="card-title">模块状态</div><p class="text-dim">无法获取状态：${esc(err.message || '')}</p>`;
    }
  };
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
