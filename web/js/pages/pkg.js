// pkg package management — list installed packages, view package details.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { t } from '../i18n/index.js';

let _pkgFilter = 'all';
let _allPackages = [];

// ===== Package list page =====

export async function renderPackages(app) {
  renderLayout(app, '/pkg', `
    <div class="page-header">
      <h1>${t('nav.packages')}</h1>
      <p>${t('pkg.subtitle')}</p>
    </div>
    <div class="toolbar">
      <div class="filter-group" id="pkg-filter-group">
        <button class="filter-btn active" data-val="all">${t('common.all')}</button>
        <button class="filter-btn" data-val="manual">${t('pkg.manual')}</button>
        <button class="filter-btn" data-val="automatic">${t('pkg.automatic')}</button>
      </div>
      <input type="text" id="pkg-search" class="filter-input" placeholder="${t('pkg.searchPh')}" oninput="window.__fwpPkgSearch()" />
      <span id="pkg-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpPkgReload()">${t('common.refresh')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>${t('common.name')}</th>
          <th>${t('pkg.version')}</th>
          <th>${t('common.description')}</th>
          <th>${t('common.size')}</th>
          <th>${t('common.status')}</th>
        </tr></thead>
        <tbody id="pkg-tbody">
          <tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);

  _pkgFilter = 'all';
  bindPkgFilters();
  await loadPackages();
}

function bindPkgFilters() {
  const group = document.getElementById('pkg-filter-group');
  if (!group) return;
  group.addEventListener('click', (ev) => {
    const btn = ev.target.closest('.filter-btn');
    if (!btn) return;
    group.querySelectorAll('.filter-btn').forEach((b) => b.classList.remove('active'));
    btn.classList.add('active');
    _pkgFilter = btn.dataset.val;
    loadPackages();
  });
}

async function loadPackages() {
  const tbody = document.getElementById('pkg-tbody');
  const countEl = document.getElementById('pkg-count');
  const searchEl = document.getElementById('pkg-search');
  if (searchEl) searchEl.value = '';

  try {
    _allPackages = await api.get(`/api/pkg/packages?filter=${_pkgFilter}`);
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="5" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }
  renderPkgRows(_allPackages, countEl);
}

function renderPkgRows(packages, countEl) {
  const tbody = document.getElementById('pkg-tbody');
  if (countEl) countEl.textContent = t('pkg.count', { n: packages.length });
  if (!packages.length) {
    tbody.innerHTML = `<tr><td colspan="5" class="empty">${t('pkg.noPackages')}</td></tr>`;
    return;
  }
  tbody.innerHTML = packages.map((p) => `
    <tr style="cursor:pointer;" onclick="location.hash='#/pkg/${escAttr(p.name)}'">
      <td class="mono"><strong>${esc(p.name)}</strong></td>
      <td class="mono text-dim">${esc(p.version)}</td>
      <td><div class="cell-wrap">${esc(p.comment) || '<span class="text-dim">—</span>'}</div></td>
      <td class="mono">${esc(p.size)}</td>
      <td>${p.automatic
        ? `<span class="badge badge-dim">${t('pkg.automatic')}</span>`
        : `<span class="badge badge-success">${t('pkg.manual')}</span>`}
      </td>
    </tr>`).join('');
}

window.__fwpPkgSearch = () => {
  const q = (document.getElementById('pkg-search')?.value || '').toLowerCase();
  const list = q
    ? _allPackages.filter((p) =>
        p.name.toLowerCase().includes(q) ||
        (p.comment || '').toLowerCase().includes(q) ||
        (p.origin || '').toLowerCase().includes(q))
    : _allPackages;
  renderPkgRows(list, document.getElementById('pkg-count'));
};

window.__fwpPkgReload = () => {
  const tbody = document.getElementById('pkg-tbody');
  if (tbody) tbody.innerHTML = `<tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>`;
  loadPackages();
};

// ===== Package detail page =====

let _detailTab = 'info';

export async function renderPackageDetail(app, hashPath) {
  const name = hashPath.replace(/^\/pkg\//, '');

  renderLayout(app, '/pkg', `
    <div class="page-header">
      <div class="flex">
        <a href="#/pkg" class="btn-secondary btn-sm">${t('common.navBack')}</a>
        <h1 id="pkg-detail-title">${esc(name)}</h1>
      </div>
      <p>${t('pkg.detailSubtitle')}</p>
    </div>
    <div id="pkg-detail-body"><div class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>
  `);

  _detailTab = 'info';
  const el = document.getElementById('pkg-detail-body');

  let info;
  try {
    info = await api.get(`/api/pkg/packages/${encodeURIComponent(name)}`);
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
    return;
  }

  document.getElementById('pkg-detail-title').textContent = `${info.name}-${info.version}`;
  renderDetailTabs(el, info, name);
}

function renderDetailTabs(el, info, name) {
  el.innerHTML = `
    <div class="toolbar" style="margin-bottom:16px;">
      <div class="filter-group" id="pkg-detail-tabs">
        <button class="filter-btn active" data-val="info">${t('pkg.tabInfo')}</button>
        <button class="filter-btn" data-val="deps">${t('pkg.tabDeps')}</button>
        <button class="filter-btn" data-val="files">${t('pkg.tabFiles')}</button>
      </div>
    </div>
    <div id="pkg-tab-content"></div>
  `;

  const tabGroup = document.getElementById('pkg-detail-tabs');
  tabGroup.addEventListener('click', (ev) => {
    const btn = ev.target.closest('.filter-btn');
    if (!btn) return;
    tabGroup.querySelectorAll('.filter-btn').forEach((b) => b.classList.remove('active'));
    btn.classList.add('active');
    _detailTab = btn.dataset.val;
    renderTabContent(info, name);
  });

  renderTabContent(info, name);
}

function renderTabContent(info, name) {
  const el = document.getElementById('pkg-tab-content');
  if (_detailTab === 'info') {
    el.innerHTML = renderInfoTab(info);
  } else if (_detailTab === 'deps') {
    el.innerHTML = renderDepsTab(info);
  } else if (_detailTab === 'files') {
    el.innerHTML = `<div class="card" style="padding:0;"><div id="pkg-files" class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>`;
    loadFiles(name);
  }
}

function renderInfoTab(info) {
  const licenses = info.licenses.length
    ? info.licenses.map((l) => `<span class="badge badge-dim">${esc(l)}</span>`).join(' ')
    : '<span class="text-dim">—</span>';
  const cats = info.categories.length
    ? info.categories.map((c) => `<span class="badge badge-dim">${esc(c)}</span>`).join(' ')
    : '<span class="text-dim">—</span>';

  const flags = [
    info.automatic ? `<span class="badge badge-dim">${t('pkg.automatic')}</span>` : `<span class="badge badge-success">${t('pkg.manual')}</span>`,
    info.locked ? `<span class="badge badge-warn">${t('pkg.locked')}</span>` : '',
    info.vital ? `<span class="badge badge-success">${t('pkg.vital')}</span>` : '',
  ].filter(Boolean).join(' ');

  return `
    <div class="card">
      <div class="flex" style="justify-content:space-between; margin-bottom: 12px;">
        <div><strong style="font-size:18px;">${esc(info.name)}-${esc(info.version)}</strong></div>
        <div>${flags}</div>
      </div>
      ${esc(info.comment) ? `<p style="margin-bottom:16px; font-size:15px; color:var(--text-dim);">${esc(info.comment)}</p>` : ''}
      ${esc(info.description) ? `<div style="margin-bottom:16px;"><div class="card-title">${t('common.description')}</div><p style="white-space:pre-wrap;">${esc(info.description)}</p></div>` : ''}
      ${info.messages && info.messages.length ? `
      <div class="card" style="border-color:var(--warn); margin-bottom:16px;">
        <div class="card-title" style="color:var(--warn);">${t('pkg.messages')}</div>
        ${info.messages.map((m) => `<pre style="white-space:pre-wrap; font-family:inherit; margin:0;">${esc(m)}</pre>`).join('<hr style="border-color:var(--border); margin:8px 0;">')}
      </div>` : ''}
      <table class="kv-table">
        <tr><td>${t('pkg.origin')}</td><td class="mono">${esc(info.origin)}</td></tr>
        <tr><td>${t('pkg.version')}</td><td class="mono">${esc(info.version)}</td></tr>
        <tr><td>${t('common.size')}</td><td>${fmtBytes(info.size_bytes)}</td></tr>
        <tr><td>${t('pkg.prefix')}</td><td class="mono">${esc(info.prefix)}</td></tr>
        <tr><td>${t('pkg.homepage')}</td><td><a href="${escAttr(info.homepage)}" target="_blank" rel="noopener">${esc(info.homepage)}</a></td></tr>
        <tr><td>${t('pkg.maintainer')}</td><td>${esc(info.maintainer)}</td></tr>
        <tr><td>${t('pkg.repository')}</td><td>${esc(info.repository)}</td></tr>
        <tr><td>ABI</td><td class="mono">${esc(info.abi)}</td></tr>
        <tr><td>${t('pkg.arch')}</td><td class="mono">${esc(info.arch)}</td></tr>
        <tr><td>${t('pkg.installed')}</td><td>${fmtDate(info.install_timestamp)}</td></tr>
        <tr><td>${t('pkg.categories')}</td><td>${cats}</td></tr>
        <tr><td>${t('pkg.licenses')}</td><td>${licenses}</td></tr>
      </table>
    </div>
  `;
}

function renderDepsTab(info) {
  const deps = info.dependencies || [];
  const rdeps = info.reverse_dependencies || [];

  return `
    <div style="display:grid; grid-template-columns: 1fr 1fr; gap:16px;">
      <div class="card">
        <div class="card-title">${t('pkg.dependsOn')} (${deps.length})</div>
        ${deps.length ? `
        <table>
          <thead><tr><th>${t('common.name')}</th><th>${t('pkg.version')}</th></tr></thead>
          <tbody>
            ${deps.map((d) => `
              <tr style="cursor:pointer;" onclick="location.hash='#/pkg/${escAttr(d.name)}'">
                <td class="mono">${esc(d.name)}</td>
                <td class="mono text-dim">${esc(d.version)}</td>
              </tr>`).join('')}
          </tbody>
        </table>` : `<div class="empty">${t('pkg.noDeps')}</div>`}
      </div>
      <div class="card">
        <div class="card-title">${t('pkg.requiredBy')} (${rdeps.length})</div>
        ${rdeps.length ? `
        <table>
          <thead><tr><th>${t('common.name')}</th><th>${t('pkg.version')}</th></tr></thead>
          <tbody>
            ${rdeps.map((d) => `
              <tr style="cursor:pointer;" onclick="location.hash='#/pkg/${escAttr(d.name)}'">
                <td class="mono">${esc(d.name)}</td>
                <td class="mono text-dim">${esc(d.version)}</td>
              </tr>`).join('')}
          </tbody>
        </table>` : `<div class="empty">${t('pkg.noRdeps')}</div>`}
      </div>
    </div>
  `;
}

async function loadFiles(name) {
  const el = document.getElementById('pkg-files');
  try {
    const files = await api.get(`/api/pkg/packages/${encodeURIComponent(name)}/files`);
    if (!files.length) {
      el.className = 'empty';
      el.innerHTML = t('pkg.noFiles');
      return;
    }
    el.className = '';
    el.innerHTML = `
      <table>
        <thead><tr>
          <th>${t('pkg.filePath')}</th>
          <th>${t('common.owner')}</th>
          <th>${t('common.group')}</th>
          <th>${t('common.permissions')}</th>
        </tr></thead>
        <tbody>
          ${files.map((f) => `
            <tr>
              <td class="mono">${esc(f.path)}</td>
              <td>${esc(f.owner || '—')}</td>
              <td>${esc(f.group || '—')}</td>
              <td class="mono">${esc(f.mode || '—')}</td>
            </tr>`).join('')}
        </tbody>
      </table>
    `;
  } catch (e) {
    el.className = 'empty';
    el.innerHTML = t('common.loadFailed', { msg: esc(e.message || '') });
  }
}

// ---- Utilities ----

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

function fmtBytes(n) {
  if (!n || n <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function fmtDate(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleString();
}
