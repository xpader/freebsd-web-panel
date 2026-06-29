// RC Config — list / add / edit / delete rc.conf variables (via sysrc).

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { formModal } from '../ui/formModal.js';
import { t } from '../i18n/index.js';

let _allVars = [];

export async function renderRcconf(app) {
  renderLayout(app, '/rcconf', `
    <div class="page-header">
      <h1>${t('rcconf.title')}</h1>
      <p>${t('rcconf.subtitle')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="rc-filter" class="filter-input" placeholder="${t('rcconf.filter')}" oninput="window.__fwpRcFilter()" />
      <span id="rc-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpRcAdd()">${t('rcconf.add')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('common.key')}</th><th>${t('common.value')}</th><th>${t('common.actions')}</th></tr></thead>
        <tbody id="rc-tbody">
          <tr><td colspan="3" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);
  await loadVars();
}

async function loadVars() {
  const tbody = document.getElementById('rc-tbody');
  const countEl = document.getElementById('rc-count');
  try {
    _allVars = await api.get('/api/rcconf');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="3" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }
  renderRows(_allVars, countEl);
}

function renderRows(vars, countEl) {
  const tbody = document.getElementById('rc-tbody');
  if (countEl) countEl.textContent = t('rcconf.count', { n: vars.length });
  if (!vars.length) {
    tbody.innerHTML = `<tr><td colspan="3" class="empty">${t('rcconf.noVars')}</td></tr>`;
    return;
  }
  tbody.innerHTML = vars.map((v) => `
    <tr>
      <td class="mono"><strong>${esc(v.key)}</strong></td>
      <td class="mono"><div class="cell-wrap">${esc(v.value) || '<span class="text-dim">—</span>'}</div></td>
      <td>
        <button class="btn-secondary btn-sm" onclick="window.__fwpRcEdit('${escAttr(v.key)}')">${t('common.edit')}</button>
        <button class="btn-danger btn-sm" onclick="window.__fwpRcDel('${escAttr(v.key)}')">${t('common.delete')}</button>
      </td>
    </tr>`).join('');
}

window.__fwpRcFilter = () => {
  const q = (document.getElementById('rc-filter')?.value || '').toLowerCase();
  const list = q
    ? _allVars.filter((v) => v.key.toLowerCase().includes(q) || v.value.toLowerCase().includes(q))
    : _allVars;
  renderRows(list, document.getElementById('rc-count'));
};

window.__fwpRcAdd = async () => {
  const result = await formModal(t('rcconf.addTitle'), [
    { key: 'key', label: t('common.key'), placeholder: t('rcconf.keyPlaceholder'), required: true },
    { key: 'value', label: t('common.value'), placeholder: 'YES' },
  ], t('rcconf.add'));
  if (!result) return;
  api.put('/api/rcconf', { key: result.key.trim(), value: result.value }).then(() => {
    toast(t('rcconf.added'));
    loadVars();
  }).catch((e) => toast(e.message || t('common.saveFailed', { msg: '' }), 'error'));
};

window.__fwpRcEdit = async (key) => {
  const existing = _allVars.find((v) => v.key === key);
  const result = await formModal(t('rcconf.editTitle', { key }), [
    { key: 'value', label: t('common.value'), value: existing ? existing.value : '', placeholder: 'YES' },
  ], t('common.save'));
  if (!result) return;
  api.put('/api/rcconf', { key, value: result.value }).then(() => {
    toast(t('rcconf.saved'));
    loadVars();
  }).catch((e) => toast(e.message || t('common.saveFailed', { msg: '' }), 'error'));
};

window.__fwpRcDel = async (key) => {
  if (!await confirmDialog(t('rcconf.deleteTitle'), t('rcconf.deleteConfirm', { key }))) return;
  api.del(`/api/rcconf?key=${encodeURIComponent(key)}`).then(() => {
    toast(t('rcconf.deleted'));
    loadVars();
  }).catch((e) => toast(e.message || t('common.deleteFailed'), 'error'));
};

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

// Escape for use inside an HTML single-quoted attribute (e.g. onclick="...('KEY')").
function escAttr(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/'/g, '&#39;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}
