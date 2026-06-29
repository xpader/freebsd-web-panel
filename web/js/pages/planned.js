// Generic "planned" placeholder page for FreeBSD management modules.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

// `key` is the module id used for i18n keys under `planned.<key>`.
// `labelKey` resolves the page title (and is reused for the nav entry).
export function makePlannedPage({ key, labelKey }) {
  return async function (app) {
    const path = `/${key}`;
    renderLayout(app, path, `
      <div class="page-header">
        <h1>${t(labelKey)}</h1>
        <p>${t(`planned.${key}.desc`)}</p>
      </div>
      <div class="card" id="module-status-card">
        <div class="empty"><span class="spinner"></span> ${t('planned.checking')}</div>
      </div>
      <div class="card">
        <div class="card-title">${t('planned.plan')}</div>
        <p class="text-dim">${t(`planned.${key}.detail`)}</p>
        <p class="text-dim mt-8">${t('planned.skeletonNote')}</p>
      </div>
    `);

    const card = document.getElementById('module-status-card');
    try {
      const status = await api.get(`/api${path}`);
      card.innerHTML = `
        <div class="card-title">${t('common.moduleStatus')}</div>
        <div class="flex">
          <span class="badge ${status.status === 'planned' ? 'badge-warn' : 'badge-success'}">${esc(status.status)}</span>
          <span class="text-dim">${esc(status.message)}</span>
        </div>`;
    } catch (err) {
      card.innerHTML = `<div class="card-title">${t('common.moduleStatus')}</div><p class="text-dim">${t('planned.getStatusFailed', { msg: err.message || '' })}</p>`;
    }
  };
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
