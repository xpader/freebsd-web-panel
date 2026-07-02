// Sysctl — browse kernel state variables with descriptions and modification status.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { formModal } from '../ui/formModal.js';
import { t } from '../i18n/index.js';

const PAGE_SIZE = 100;

let _allEntries = [];
let _filtered = [];
let _page = 0;
let _modFilter = 'modified';   // 'modified' | 'all'
let _wrFilter = 'writable';    // 'writable' | 'readonly' | 'all'

export async function renderSysctl(app) {
  renderLayout(app, '/sysctl', `
    <div class="page-header">
      <h1>${t('sysctl.title')}</h1>
      <p>${t('sysctl.subtitle')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="sysctl-filter" class="filter-input" placeholder="${t('sysctl.filter')}" oninput="window.__fwpSysctlFilter()" />
      <div class="filter-group" id="sysctl-filter-mod">
        <button class="filter-btn active" data-val="modified">${t('sysctl.modified')}</button>
        <button class="filter-btn" data-val="all">${t('common.all')}</button>
      </div>
      <div class="filter-group" id="sysctl-filter-wr">
        <button class="filter-btn active" data-val="writable">${t('sysctl.writable')}</button>
        <button class="filter-btn" data-val="readonly">${t('sysctl.readonly')}</button>
        <button class="filter-btn" data-val="all">${t('common.all')}</button>
      </div>
      <span id="sysctl-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpSysctlRefresh()">${t('common.refresh')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>${t('common.name')}</th>
          <th>${t('common.value')}</th>
          <th>${t('common.type')}</th>
          <th>${t('common.description')}</th>
          <th>${t('common.actions')}</th>
        </tr></thead>
        <tbody id="sysctl-tbody">
          <tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
    <div id="sysctl-pagination" class="pagination"></div>
  `);

  _page = 0;
  _modFilter = 'modified';
  _wrFilter = 'writable';
  bindFilterGroup('sysctl-filter-mod', (v) => { _modFilter = v; });
  bindFilterGroup('sysctl-filter-wr', (v) => { _wrFilter = v; });
  await loadEntries();
}

async function loadEntries() {
  const tbody = document.getElementById('sysctl-tbody');
  const countEl = document.getElementById('sysctl-count');
  try {
    _allEntries = await api.get('/api/sysctl');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="5" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }
  applyFilters();
}

function applyFilters() {
  const q = (document.getElementById('sysctl-filter')?.value || '').toLowerCase();
  _filtered = _allEntries.filter((e) => {
    if (_modFilter === 'modified' && !e.modified) return false;
    if (_wrFilter === 'writable' && !e.writable) return false;
    if (_wrFilter === 'readonly' && e.writable) return false;
    if (!q) return true;
    return (
      e.name.toLowerCase().includes(q) ||
      (e.value || '').toLowerCase().includes(q) ||
      (e.description || '').toLowerCase().includes(q)
    );
  });
  if (_page * PAGE_SIZE >= _filtered.length) _page = 0;
  renderPage();
}

function renderPage() {
  const tbody = document.getElementById('sysctl-tbody');
  const countEl = document.getElementById('sysctl-count');
  const pagEl = document.getElementById('sysctl-pagination');

  if (countEl) {
    const shown = Math.min(_filtered.length, PAGE_SIZE);
    const from = _filtered.length === 0 ? 0 : _page * PAGE_SIZE + 1;
    const to = _page * PAGE_SIZE + shown;
    countEl.textContent = t('sysctl.count', {
      total: _allEntries.length,
      range: _filtered.length ? `${from}–${to}` : '0',
      filtered: _filtered.length,
    });
  }

  if (!_filtered.length) {
    tbody.innerHTML = `<tr><td colspan="5" class="empty">${t('sysctl.noResults')}</td></tr>`;
    pagEl.innerHTML = '';
    return;
  }

  const start = _page * PAGE_SIZE;
  const slice = _filtered.slice(start, start + PAGE_SIZE);
  tbody.innerHTML = slice.map((e, i) => {
    const idx = _filtered.indexOf(e) >= 0 ? _allEntries.indexOf(e) : start + i;
    const realIdx = start + i;
    const actions = [];
    if (e.writable && e.value !== null) {
      actions.push(`<button class="btn-secondary btn-sm" onclick="window.__fwpSysctlEdit(${realIdx})">${t('common.edit')}</button>`);
    }
    if (e.modified) {
      actions.push(`<button class="btn-danger btn-sm" onclick="window.__fwpSysctlReset(${realIdx})">${t('sysctl.reset')}</button>`);
    }
    if (!actions.length) actions.push('<span class="text-dim">—</span>');
    return `
    <tr${e.modified ? ' class="row-modified"' : ''}>
      <td class="mono">
        <strong>${esc(e.name)}</strong>
        ${e.modified ? `<span class="badge-modified" title="${t('sysctl.modifiedHint')}">${t('sysctl.modified')}</span>` : ''}
        ${e.writable ? `<span class="badge-writable" title="${t('sysctl.writableHint')}">${t('sysctl.writable')}</span>` : ''}
      </td>
      <td class="mono"><div class="cell-ellipsis" title="${escAttr(e.value || '')}">${esc(truncate(e.value || '')) || '<span class="text-dim">—</span>'}</div></td>
      <td>${e.type ? `<span class="badge-type">${esc(e.type)}</span>` : '<span class="text-dim">—</span>'}</td>
      <td><div class="cell-wrap text-dim">${esc(e.description || '') || '<span class="text-dim">—</span>'}</div></td>
      <td>${actions.join(' ')}</td>
    </tr>`;
  }).join('');

  renderPagination(pagEl);
}

function renderPagination(pagEl) {
  const totalPages = Math.ceil(_filtered.length / PAGE_SIZE);
  if (totalPages <= 1) {
    pagEl.innerHTML = '';
    return;
  }

  const cur = _page + 1;
  const parts = [];

  parts.push(`<button class="btn-secondary btn-sm" ${cur === 1 ? 'disabled' : ''} onclick="window.__fwpSysctlPage(${_page - 1})">${t('sysctl.prev')}</button>`);

  const maxButtons = 9;
  let s = Math.max(0, _page - Math.floor(maxButtons / 2));
  let e = Math.min(totalPages, s + maxButtons);
  s = Math.max(0, e - maxButtons);

  if (s > 0) {
    parts.push(`<button class="btn-secondary btn-sm" onclick="window.__fwpSysctlPage(0)">1</button>`);
    if (s > 1) parts.push('<span class="text-dim">…</span>');
  }
  for (let i = s; i < e; i++) {
    parts.push(`<button class="btn-secondary btn-sm ${i === _page ? 'active' : ''}" onclick="window.__fwpSysctlPage(${i})">${i + 1}</button>`);
  }
  if (e < totalPages) {
    if (e < totalPages - 1) parts.push('<span class="text-dim">…</span>');
    parts.push(`<button class="btn-secondary btn-sm" onclick="window.__fwpSysctlPage(${totalPages - 1})">${totalPages}</button>`);
  }

  parts.push(`<button class="btn-secondary btn-sm" ${cur === totalPages ? 'disabled' : ''} onclick="window.__fwpSysctlPage(${_page + 1})">${t('sysctl.next')}</button>`);

  pagEl.innerHTML = parts.join('');
}

window.__fwpSysctlFilter = () => {
  _page = 0;
  applyFilters();
};

function bindFilterGroup(id, setter) {
  const group = document.getElementById(id);
  if (!group) return;
  group.addEventListener('click', (ev) => {
    const btn = ev.target.closest('.filter-btn');
    if (!btn) return;
    group.querySelectorAll('.filter-btn').forEach((b) => b.classList.remove('active'));
    btn.classList.add('active');
    setter(btn.dataset.val);
    _page = 0;
    applyFilters();
  });
}

window.__fwpSysctlPage = (p) => {
  _page = p;
  renderPage();
  document.querySelector('.main-content')?.scrollTo({ top: 0, behavior: 'smooth' });
};

window.__fwpSysctlRefresh = async () => {
  _page = 0;
  await loadEntries();
};

window.__fwpSysctlEdit = async (idx) => {
  const e = _filtered[idx];
  if (!e) return;
  const result = await formModal(
    t('sysctl.editTitle', { name: e.name }),
    [
      { key: 'value', label: t('common.value'), value: e.value || '', placeholder: e.value || '' },
    ],
    t('common.save'),
  );
  if (!result) return;

  const persist = await confirmDialog(
    t('sysctl.persistTitle'),
    t('sysctl.persistConfirm', { name: e.name }),
    [{ key: 'persist', label: t('sysctl.persistCheck'), checked: true }],
  );
  if (!persist) return;

  api.put(`/api/sysctl/${encodeURIComponent(e.name)}`, {
    value: result.value,
    persist: persist.persist,
  }).then(() => {
    toast(t('sysctl.saved', { name: e.name }));
    loadEntries();
  }).catch((err) => toast(err.message || t('common.saveFailed', { msg: '' }), 'error'));
};

window.__fwpSysctlReset = async (idx) => {
  const e = _filtered[idx];
  if (!e) return;
  if (!await confirmDialog(
    t('sysctl.resetTitle'),
    t('sysctl.resetConfirm', { name: e.name }),
  )) return;

  api.del(`/api/sysctl/${encodeURIComponent(e.name)}`).then(() => {
    toast(t('sysctl.resetDone', { name: e.name }));
    loadEntries();
  }).catch((err) => toast(err.message || t('common.operationFailed'), 'error'));
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

function truncate(s, max = 120) {
  return s.length > max ? s.slice(0, max) + '…' : s;
}
