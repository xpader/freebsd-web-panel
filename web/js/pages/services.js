// Services — list rc.d services with enabled/running status and control actions.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { t } from '../i18n/index.js';

let _allServices = [];

export async function renderServices(app) {
  renderLayout(app, '/services', `
    <div class="page-header">
      <h1>${t('svc.title')}</h1>
      <p>${t('svc.subtitle')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="svc-filter" class="filter-input" placeholder="${t('svc.filter')}" oninput="window.__fwpSvcFilter()" />
      <span id="svc-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpSvcReload()">${t('common.refresh')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>${t('common.name')}</th>
          <th>${t('svc.location')}</th>
          <th>${t('common.description')}</th>
          <th>${t('common.enabled')}</th>
          <th>${t('common.status')}</th>
          <th>${t('common.actions')}</th>
        </tr></thead>
        <tbody id="svc-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);
  await loadServices();
}

async function loadServices() {
  const tbody = document.getElementById('svc-tbody');
  const countEl = document.getElementById('svc-count');
  try {
    _allServices = await api.get('/api/services');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }
  renderRows(_allServices, countEl);
}

function renderRows(services, countEl) {
  const tbody = document.getElementById('svc-tbody');
  if (countEl) countEl.textContent = t('svc.count', { n: services.length });
  if (!services.length) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('svc.noServices')}</td></tr>`;
    return;
  }
  tbody.innerHTML = services.map((s) => `
    <tr>
      <td class="mono"><strong>${esc(s.name)}</strong></td>
      <td><span class="badge ${s.source === 'system' ? 'badge-dim' : ''}">${s.source === 'system' ? t('svc.system') : t('svc.local')}</span></td>
      <td><div class="cell-wrap">${esc(s.description || '') || '<span class="text-dim">—</span>'}</div></td>
      <td>${enabledBadge(s.enabled)}</td>
      <td>${statusBadge(s.running, s.enabled)}</td>
      <td>
        <div class="btn-group">
          <button class="btn-secondary btn-sm" onclick="window.__fwpSvcAction('${escAttr(s.name)}','start')" ${s.running ? 'disabled' : ''}>${t('svc.start')}</button>
          <button class="btn-secondary btn-sm" onclick="window.__fwpSvcAction('${escAttr(s.name)}','stop')" ${!s.running ? 'disabled' : ''}>${t('svc.stop')}</button>
          <button class="btn-secondary btn-sm" onclick="window.__fwpSvcAction('${escAttr(s.name)}','restart')">${t('svc.restart')}</button>
        </div>
      </td>
    </tr>`).join('');
}

function enabledBadge(enabled) {
  return enabled
    ? `<span class="badge badge-success">${t('common.enabled')}</span>`
    : `<span class="badge badge-dim">${t('common.disabled')}</span>`;
}

function statusBadge(running, enabled) {
  if (running) return `<span class="badge badge-success">${t('svc.running')}</span>`;
  if (enabled) return `<span class="badge badge-warn">${t('svc.stopped')}</span>`;
  return `<span class="badge badge-dim">${t('svc.stopped')}</span>`;
}

window.__fwpSvcFilter = () => {
  const q = (document.getElementById('svc-filter')?.value || '').toLowerCase();
  const list = q
    ? _allServices.filter((s) =>
        s.name.toLowerCase().includes(q) ||
        (s.description || '').toLowerCase().includes(q))
    : _allServices;
  renderRows(list, document.getElementById('svc-count'));
};

window.__fwpSvcReload = () => {
  const tbody = document.getElementById('svc-tbody');
  if (tbody) tbody.innerHTML = `<tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>`;
  loadServices();
};

window.__fwpSvcAction = async (name, action) => {
  try {
    await api.post(`/api/services/${encodeURIComponent(name)}/${action}`);
    toast(t('svc.actionDone', { name, action: t('svc.' + action) }));
    await loadServices();
  } catch (e) {
    toast(e.message || t('common.operationFailed'), 'error');
  }
};

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

function escAttr(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/'/g, '&#39;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}
