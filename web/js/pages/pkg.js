// pkg package management — list, search, install, delete, view details.

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
      </div>
      <input type="text" id="pkg-search" class="filter-input" placeholder="${t('pkg.filterPh')}" oninput="window.__fwpPkgSearch()" />
      <span id="pkg-count" class="text-dim"></span>
      <div class="flex">
        <button onclick="window.__fwpPkgInstallOpen()"><i class="fa-solid fa-download"></i> ${t('pkg.installBtn')}</button>
        <button onclick="window.__fwpPkgReload()"><i class="fa-solid fa-rotate-right"></i> ${t('common.refresh')}</button>
      </div>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>${t('common.name')}</th>
          <th>${t('pkg.version')}</th>
          <th>${t('common.description')}</th>
          <th>${t('common.size')}</th>
          <th>${t('common.status')}</th>
          <th>${t('common.actions')}</th>
        </tr></thead>
        <tbody id="pkg-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
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
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }
  renderPkgRows(_allPackages, countEl);
}

function renderPkgRows(packages, countEl) {
  const tbody = document.getElementById('pkg-tbody');
  if (countEl) countEl.textContent = t('pkg.count', { n: packages.length });
  if (!packages.length) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('pkg.noPackages')}</td></tr>`;
    return;
  }
  tbody.innerHTML = packages.map((p) => `
    <tr>
      <td class="mono"><strong><a href="#/pkg/${escAttr(p.name)}">${esc(p.name)}</a></strong></td>
      <td class="mono text-dim">${esc(p.version)}</td>
      <td><div class="cell-wrap">${esc(p.comment) || '<span class="text-dim">—</span>'}</div></td>
      <td class="mono">${esc(p.size)}</td>
      <td>${p.automatic
        ? `<span class="badge badge-dim">${t('pkg.automatic')}</span>`
        : `<span class="badge badge-success">${t('pkg.manual')}</span>`}
      </td>
      <td>
        <button class="btn-secondary btn-sm" onclick="window.__fwpPkgDelete('${escAttr(p.name)}')">${t('common.delete')}</button>
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
  if (tbody) tbody.innerHTML = `<tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>`;
  loadPackages();
};

// ===== Install: search modal =====

window.__fwpPkgInstallOpen = () => {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal" style="max-width:680px;">
      <h3><i class="fa-solid fa-download"></i> ${t('pkg.installTitle')}</h3>
      <div class="field" style="margin-bottom:12px;">
        <div class="flex">
          <input type="text" id="pkg-remote-search" class="filter-input" style="flex:1;" placeholder="${t('pkg.searchRemotePh')}" />
          <button onclick="window.__fwpPkgRemoteSearch()"><i class="fa-solid fa-magnifying-glass"></i> ${t('pkg.searchBtn')}</button>
        </div>
      </div>
      <div id="pkg-search-results" style="max-height:360px; overflow-y:auto;">
        <div class="empty" style="padding:20px;">${t('pkg.searchHint')}</div>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" onclick="this.closest('.modal-overlay').remove()">${t('common.close')}</button>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', (e) => { if (e.target === overlay) overlay.remove(); });

  const input = document.getElementById('pkg-remote-search');
  input.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') {
      ev.preventDefault();
      window.__fwpPkgRemoteSearch();
    }
  });
  input.focus();
};

let _installedNames = new Set();

window.__fwpPkgRemoteSearch = () => {
  const input = document.getElementById('pkg-remote-search');
  if (!input) return;
  const q = input.value.trim();
  if (!q) return;
  doRemoteSearch(q);
};

async function doRemoteSearch(q) {
  const el = document.getElementById('pkg-search-results');
  el.innerHTML = `<div class="empty" style="padding:20px;"><span class="spinner"></span> ${t('common.loading')}</div>`;
  try {
    const results = await api.get(`/api/pkg/search?q=${encodeURIComponent(q)}`);
    // Build set of installed names for marking.
    const installed = await api.get('/api/pkg/packages');
    _installedNames = new Set(installed.map((p) => p.name));

    if (!results.length) {
      el.innerHTML = `<div class="empty" style="padding:20px;">${t('pkg.noSearchResults')}</div>`;
      return;
    }
    el.innerHTML = `
      <table style="width:100%;">
        <thead><tr><th>${t('common.name')}</th><th>${t('common.description')}</th><th>${t('common.size')}</th><th style="white-space:nowrap;"></th></tr></thead>
        <tbody>
          ${results.map((r) => `
            <tr>
              <td class="mono"><strong>${esc(r.name)}</strong><br><span class="text-dim" style="font-size:11px;">${esc(r.version)}</span></td>
              <td><div class="cell-wrap">${esc(r.comment)}</div></td>
              <td class="mono text-dim">${esc(r.size)}</td>
              <td style="white-space:nowrap;">${_installedNames.has(r.name)
                ? `<span class="badge badge-dim">${t('pkg.installedBadge')}</span>`
                : `<button class="btn-secondary btn-sm" onclick="window.__fwpPkgDoInstall('${escAttr(r.name)}')">${t('pkg.installBtn')}</button>`}
              </td>
            </tr>`).join('')}
        </tbody>
      </table>
    `;
  } catch (e) {
    el.innerHTML = `<div class="empty" style="padding:20px;">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
  }
}

window.__fwpPkgDoInstall = async (name) => {
  await confirmPkgAction('install', [name]);
};

// ===== Delete =====

window.__fwpPkgDelete = async (name) => {
  await confirmPkgAction('delete', [name]);
};

async function confirmPkgAction(action, packages) {
  const result = await showPreviewConfirm(action, packages);
  if (!result) return;

  if (action === 'install') {
    const overlay = document.querySelector('.modal-overlay');
    if (overlay) overlay.remove();
  }
  await startPkgTask(action, packages);
}

async function showPreviewConfirm(action, packages) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.innerHTML = `
      <div class="modal" style="max-width:560px;">
        <h3>${esc(action === 'install'
          ? t('pkg.installConfirm', { name: packages.join(', ') })
          : t('pkg.deleteConfirm', { name: packages.join(', ') }))}</h3>
        <div id="pkg-preview-body" style="min-height:60px;">
          <div class="empty" style="padding:20px;"><span class="spinner"></span> ${t('common.loading')}</div>
        </div>
        <div class="modal-actions">
          <button class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
          <button class="btn-danger" data-act="ok" disabled>${esc(action === 'install' ? t('pkg.installBtn') : t('common.delete'))}</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);

    const okBtn = overlay.querySelector('[data-act="ok"]');
    const cancelBtn = overlay.querySelector('[data-act="cancel"]');
    cancelBtn.addEventListener('click', () => { overlay.remove(); resolve(false); });
    okBtn.addEventListener('click', () => { overlay.remove(); resolve(true); });
    overlay.addEventListener('click', (e) => { if (e.target === overlay) { overlay.remove(); resolve(false); } });

    const bodyEl = overlay.querySelector('#pkg-preview-body');

    api.post('/api/pkg/preview', { action, packages })
      .then((preview) => {
        const affected = action === 'install' ? preview.install : preview.delete;
        const deps = affected.filter((n) => !packages.includes(n));
        const targetStr = esc(packages.join(', '));

        let html = '';

        if (deps.length) {
          html += `<p class="text-dim">${esc(action === 'install'
            ? t('pkg.willInstallDeps', { n: deps.length })
            : t('pkg.willDeleteDeps', { n: deps.length }))}</p>`;
          html += `<div style="max-height:160px; overflow-y:auto; margin-top:8px;">`;
          html += deps.map((n) => `<div class="mono text-dim" style="padding:2px 0; font-size:12px;">${esc(n)}</div>`).join('');
          html += `</div>`;
        } else if (action === 'install' && affected.length === 0) {
          html += `<p class="text-dim">${esc(t('pkg.alreadyInstalled'))}</p>`;
        } else {
          html += `<p class="text-dim">${esc(action === 'install'
            ? t('pkg.noDepsToInstall')
            : t('pkg.noDepsToDelete'))}</p>`;
        }

        bodyEl.innerHTML = html;
        okBtn.disabled = false;
      })
      .catch((e) => {
        bodyEl.innerHTML = `<p>${esc(e.message || t('common.operationFailed'))}</p>`;
        okBtn.disabled = false;
        okBtn.textContent = t('common.close');
      });
  });
}

// ===== Task output modal (shared by install & delete) =====

async function startPkgTask(action, packages) {
  const endpoint = action === 'install' ? '/api/pkg/install' : '/api/pkg/delete';
  const body = action === 'delete' ? { packages } : { packages };

  let taskId;
  try {
    const res = await api.post(endpoint, body);
    taskId = res.task_id;
  } catch (e) {
    toast(e.message || t('common.operationFailed'), 'error');
    return;
  }

  showTaskModal(action, packages, taskId);
}

function showTaskModal(action, packages, taskId) {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal" style="max-width:680px;">
      <h3 id="pkg-task-title">
        <span id="pkg-task-icon" class="spinner"></span>
        ${esc(action === 'install' ? t('pkg.installing') : t('pkg.deleting'))} ${esc(packages.join(', '))}
      </h3>
      <div id="pkg-task-output" style="max-height:400px; overflow-y:auto; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius); padding:12px; margin-bottom:12px; font-family:monospace; font-size:12px; white-space:pre-wrap; word-break:break-all;"></div>
      <div class="modal-actions">
        <button id="pkg-task-close" class="btn-secondary" disabled>${t('common.close')}</button>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);

  const outputEl = document.getElementById('pkg-task-output');
  const closeBtn = document.getElementById('pkg-task-close');
  const titleEl = document.getElementById('pkg-task-title');
  closeBtn.addEventListener('click', () => overlay.remove());

  streamTask(taskId, outputEl, closeBtn, titleEl, action, packages, overlay);
}

function streamTask(taskId, outputEl, closeBtn, titleEl, action, packages, overlay) {
  const token = sessionStorage.getItem('fwp_token');
  const url = `/api/pkg/tasks/${encodeURIComponent(taskId)}/stream?token=${encodeURIComponent(token)}`;
  const es = new EventSource(url);

  const finish = (success, pkgNames) => {
    es.close();
    closeBtn.disabled = false;
    const nameStr = pkgNames || packages.join(', ');
    const doneLabel = success
      ? t(action === 'install' ? 'pkg.installDone' : 'pkg.deleteDone', { name: nameStr })
      : t(action === 'install' ? 'pkg.installFailed' : 'pkg.deleteFailed', { name: nameStr });
    const color = success ? 'var(--success)' : 'var(--danger)';
    titleEl.innerHTML = `<span style="color:${color}; font-weight:700;">${esc(doneLabel)}</span>`;
    if (success) {
      outputEl.textContent += `\n[${t('common.done')}]\n`;
    }
    toast(doneLabel, success ? 'success' : 'error');
    if (document.getElementById('pkg-tbody')) loadPackages();
  };

  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data.lines && data.lines.length) {
        const atBottom = outputEl.scrollTop + outputEl.clientHeight >= outputEl.scrollHeight - 2;
        outputEl.textContent += data.lines.join('\n') + '\n';
        if (atBottom) outputEl.scrollTop = outputEl.scrollHeight;
      }
      if (data.status && data.status !== 'running') {
        const pkgNames = Array.isArray(data.packages) ? data.packages.join(', ') : (data.packages || '');
        finish(data.status === 'done', pkgNames);
      }
    } catch {}
  };

  es.addEventListener('done', () => {
    es.close();
    closeBtn.disabled = false;
  });

  es.onerror = () => {
    es.close();
    api.get(`/api/pkg/tasks/${encodeURIComponent(taskId)}`).then((task) => {
      if (task.status !== 'running') {
        finish(task.status === 'done', task.packages.join(', '));
      } else {
        closeBtn.disabled = false;
      }
    }).catch(() => {
      closeBtn.disabled = false;
    });
  };
}

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
      ${info.messages && info.messages.length ? `
      <div class="card" style="border-color:var(--warn); margin-top:16px;">
        <div class="card-title" style="color:var(--warn);">${t('pkg.messages')}</div>
        ${info.messages.map((m) => `<pre style="white-space:pre-wrap; font-family:inherit; margin:0;">${esc(m)}</pre>`).join('<hr style="border-color:var(--border); margin:8px 0;">')}
      </div>` : ''}
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
