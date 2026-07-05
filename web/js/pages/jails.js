// Jails — list running jail containers, view details, and manage base systems.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { confirmDialog } from '../ui/confirm.js';
import { toast } from '../ui/toast.js';
import { alertDialog } from '../ui/alertDialog.js';
import { t } from '../i18n/index.js';

/// Show a loading overlay on top of a modal element, call the async onSubmit,
/// close on success or show error dialog on failure.
async function submitModal(overlay, onSubmit, result) {
  // Add a busy overlay on the modal.
  const modal = overlay.querySelector('.modal');
  if (!modal) return;
  const busy = document.createElement('div');
  busy.className = 'modal-busy';
  busy.innerHTML = '<span class="spinner"></span>';
  modal.style.position = 'relative';
  modal.appendChild(busy);

  try {
    await onSubmit(result);
    overlay.remove();
  } catch (e) {
    busy.remove();
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// ===== Jail list (with Running / All tabs) =====

let _jailTab = 'all';
const _pendingActions = new Set();

function jailState(name, running) {
  if (_pendingActions.has(`${name}:start`)) return 'starting';
  if (_pendingActions.has(`${name}:stop`)) return 'stopping';
  return running ? 'running' : 'stopped';
}

export async function renderJailsRunning(app) {
  renderLayout(app, '/jails/running', `
    <div class="page-header">
      <h1>${t('jails.title')}</h1>
      <p>${t('jails.subtitle')}</p>
    </div>
    <div class="toolbar">
      <div class="filter-group" id="jail-tab-group">
        <button class="filter-btn active" data-val="all">${t('common.all')}</button>
        <button class="filter-btn" data-val="running">${t('jails.running')}</button>
      </div>
      <span id="jail-count" class="text-dim"></span>
      <div class="flex">
        <button onclick="location.hash='#/jails/create'"><i class="fa-solid fa-plus"></i> ${t('jails.create')}</button>
        <button onclick="window.__fwpJailReload()"><i class="fa-solid fa-rotate-right"></i> ${t('common.refresh')}</button>
      </div>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>JID</th>
          <th>${t('common.name')}</th>
          <th>${t('jails.hostname')}</th>
          <th>${t('jails.path')}</th>
          <th>IP</th>
          <th>${t('common.status')}</th>
          <th>${t('common.actions')}</th>
        </tr></thead>
        <tbody id="jail-tbody">
          <tr><td colspan="7" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);

  bindJailTabs();
  await loadJails();
}

function bindJailTabs() {
  const group = document.getElementById('jail-tab-group');
  if (!group) return;
  group.addEventListener('click', (ev) => {
    const btn = ev.target.closest('.filter-btn');
    if (!btn) return;
    group.querySelectorAll('.filter-btn').forEach((b) => b.classList.remove('active'));
    btn.classList.add('active');
    _jailTab = btn.dataset.val;
    loadJails();
  });
}

async function loadJails() {
  const tbody = document.getElementById('jail-tbody');
  const countEl = document.getElementById('jail-count');
  const url = _jailTab === 'running' ? '/api/jails?running=true' : '/api/jails';

  try {
    const data = await api.get(url);
    if (countEl) countEl.textContent = t('jails.count', { n: data.length });
    if (!data.length) {
      tbody.innerHTML = `<tr><td colspan="7" class="empty">${t('jails.noJails')}</td></tr>`;
      return;
    }

    if (_jailTab === 'running') {
      // Running jails from libjail.
      tbody.innerHTML = data.map((j) => `
        <tr class="row-clickable" onclick="location.hash='#/jails/detail/${escAttr(j.name)}'">
          <td class="mono">${j.jid}</td>
          <td class="mono"><strong>${esc(j.name)}</strong></td>
          <td>${esc(j.hostname || '—')}</td>
          <td class="mono text-dim">${esc(j.path || '—')}</td>
          <td class="mono">${formatIpStr(j.ip4_addr, j.ip6_addr)}</td>
          <td>${stateBadge(jailState(j.name, true))}</td>
          <td>${actionButtons(j.name, jailState(j.name, true))}</td>
        </tr>`).join('');
    } else {
      // All jails from jail.conf.
      tbody.innerHTML = data.map((j) => `
        <tr class="row-clickable" onclick="location.hash='#/jails/detail/${escAttr(j.name)}'">
          <td class="mono text-dim">${j.jid > 0 ? j.jid : '—'}</td>
          <td class="mono"><strong>${esc(j.name)}</strong></td>
          <td>${esc(j.hostname || '—')}</td>
          <td class="mono text-dim">${esc(j.path || '—')}</td>
          <td class="mono">${formatIpStr(j.ip4_addr, j.ip6_addr)}</td>
          <td>${stateBadge(jailState(j.name, j.jid > 0))}</td>
          <td>${actionButtons(j.name, jailState(j.name, j.jid > 0))}</td>
        </tr>`).join('');
    }
  } catch (err) {
    if (countEl) countEl.textContent = '';
    tbody.innerHTML = `<tr><td colspan="7" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
  }
}

// ===== Jail actions (start/stop/delete) =====

function actionButtons(name, state) {
  const busy = state === 'starting' || state === 'stopping';
  if (busy) {
    return `<div class="btn-group" onclick="event.stopPropagation()">
      <button class="btn-secondary btn-sm" disabled>${t('jails.start')}</button>
      <button class="btn-secondary btn-sm" disabled>${t('jails.stop')}</button>
      <button class="btn-danger btn-sm" disabled>${t('common.delete')}</button>
    </div>`;
  }
  const running = state === 'running';
  const startBtn = running
    ? `<button class="btn-secondary btn-sm" disabled>${t('jails.start')}</button>`
    : `<button class="btn-secondary btn-sm" onclick="window.__fwpJailAction('${escAttr(name)}','start')">${t('jails.start')}</button>`;
  const stopBtn = running
    ? `<button class="btn-secondary btn-sm" onclick="window.__fwpJailAction('${escAttr(name)}','stop')">${t('jails.stop')}</button>`
    : `<button class="btn-secondary btn-sm" disabled>${t('jails.stop')}</button>`;
  const delBtn = `<button class="btn-danger btn-sm" onclick="window.__fwpJailDelete('${escAttr(name)}')">${t('common.delete')}</button>`;
  return `<div class="btn-group" onclick="event.stopPropagation()">${startBtn}${stopBtn}${delBtn}</div>`;
}

window.__fwpJailAction = async (name, action) => {
  _pendingActions.add(`${name}:${action}`);
  await loadJails();

  try {
    await api.post(`/api/jails/${encodeURIComponent(name)}/${action}`);
    toast(t('jails.actionDone', { name, action: t('jails.' + action) }));
  } catch (e) {
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    _pendingActions.delete(`${name}:${action}`);
    await loadJails();
  }
};

window.__fwpJailDelete = async (name) => {
  const result = await confirmDialog(
    t('jails.deleteJail'),
    t('jails.deleteConfirm', { name }),
    [{ key: 'removeFiles', label: t('jails.deleteFiles'), checked: false }],
  );
  if (!result || !result.confirmed) return;
  try {
    const qs = result.removeFiles ? '?remove_files=true' : '';
    await api.del(`/api/jails/${encodeURIComponent(name)}${qs}`);
    toast(t('jails.deleted'));
    await loadJails();
  } catch (e) {
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
};

// ===== Create jail =====

// ===== Jail create page =====

export async function renderJailCreate(app) {
  let bases = [];
  try { bases = await api.get('/api/jails/bases'); } catch {}

  const baseOptions = bases.map((b) =>
    `<option value="${escAttr(b.name)}">${esc(b.name)} (${b.type})</option>`
  ).join('');

  renderLayout(app, '/jails/running', `
    <div class="page-header">
      <div class="flex">
        <a href="#/jails/running" class="btn-secondary btn-sm">${t('common.navBack')}</a>
        <h1>${t('jails.createTitle')}</h1>
      </div>
    </div>
    <form id="create-jail-form">
      <div class="card">
        <div class="card-title">${t('jails.basicInfo')}</div>
        <div class="form-row">
          <label>${t('jails.jailName')} <span style="color:var(--danger)">*</span></label>
          <input type="text" name="name" id="cj-name" required placeholder="web01" />
        </div>
        <div class="form-row">
          <label>${t('jails.hostname')}</label>
          <input type="text" name="hostname" placeholder="${t('jails.hostnamePh')}" />
        </div>
      </div>

      <div class="card">
        <div class="card-title">${t('jails.locationType')}</div>
        <div class="form-row">
          <label>${t('jails.locationType')} <span style="color:var(--danger)">*</span></label>
          <select name="location_type" id="cj-loc-type" required>
            <option value="">${t('common.pleaseSelect')}</option>
            <option value="directory">${t('jails.locDirectory')}</option>
            <option value="base">${t('jails.locBase')}</option>
          </select>
        </div>

        <div id="cj-dir-fields" style="display:none;">
          <div class="form-row">
            <label>${t('jails.path')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="dir_path" id="cj-dir-path" placeholder="/jails/web01" />
          </div>
        </div>

        <div id="cj-base-fields" style="display:none;">
          <div class="form-row">
            <label>${t('jails.selectBase')} <span style="color:var(--danger)">*</span></label>
            <select name="base_name" id="cj-base-name">
              <option value="">${t('common.pleaseSelect')}</option>
              ${baseOptions}
            </select>
          </div>
          <div id="cj-zfs-base-fields" style="display:none;">
            <div class="form-row">
              <label>${t('jails.cloneSnapshot')} <span style="color:var(--danger)">*</span></label>
              <select name="snapshot" id="cj-snapshot"></select>
            </div>
            <div class="form-row">
              <label>${t('jails.targetDataset')}</label>
              <input type="text" name="target_dataset" id="cj-dataset" placeholder="${t('jails.datasetDefault')}" />
            </div>
            <div class="form-row">
              <label>${t('jails.mountPoint')}</label>
              <input type="text" name="base_path" id="cj-mountpoint" placeholder="/jails/web01" />
            </div>
          </div>
          <div id="cj-sfs-base-fields" style="display:none;">
            <div class="form-row">
              <label>${t('jails.targetLocation')}</label>
              <input type="text" name="sfs_path" id="cj-sfs-path" placeholder="/jails/web01" />
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">${t('common.network')}</div>
        <div class="form-row">
          <label>${t('jails.networkInterface')}</label>
          <input type="text" name="interface" placeholder="bge0" />
        </div>
        <div class="form-row">
          <label>IPv4</label>
          <input type="text" name="ip4" placeholder="${t('jails.ip4Ph')}" />
        </div>
        <div class="form-row">
          <label>IPv6</label>
          <input type="text" name="ip6" placeholder="${t('jails.ip6Ph')}" />
        </div>
      </div>

      <div class="form-actions-bar">
        <a href="#/jails/running" class="btn btn-secondary">${t('common.cancel')}</a>
        <button type="submit" id="cj-submit">${t('common.confirm')}</button>
      </div>
    </form>
  `);

  const form = document.getElementById('create-jail-form');
  const nameInput = document.getElementById('cj-name');
  const locType = document.getElementById('cj-loc-type');
  const dirFields = document.getElementById('cj-dir-fields');
  const baseFields = document.getElementById('cj-base-fields');
  const baseSel = document.getElementById('cj-base-name');
  const zfsBaseFields = document.getElementById('cj-zfs-base-fields');
  const sfsBaseFields = document.getElementById('cj-sfs-base-fields');
  const snapSel = document.getElementById('cj-snapshot');
  const datasetInput = document.getElementById('cj-dataset');
  const mountInput = document.getElementById('cj-mountpoint');
  const sfsPathInput = document.getElementById('cj-sfs-path');
  const submitBtn = document.getElementById('cj-submit');

  const _bases = bases;

  const updateDefaults = () => {
    const name = nameInput.value.trim();
    if (mountInput && !mountInput.value && name) mountInput.placeholder = `/jails/${name}`;
    if (sfsPathInput && !sfsPathInput.value && name) sfsPathInput.placeholder = `/jails/${name}`;
    if (dirFields.style.display !== 'none') {
      const dp = document.getElementById('cj-dir-path');
      if (dp && !dp.value && name) dp.placeholder = `/jails/${name}`;
    }
  };
  nameInput.addEventListener('input', updateDefaults);

  locType.addEventListener('change', () => {
    const isDir = locType.value === 'directory';
    const isBase = locType.value === 'base';
    dirFields.style.display = isDir ? '' : 'none';
    baseFields.style.display = isBase ? '' : 'none';
    updateDefaults();
  });

  baseSel.addEventListener('change', () => {
    const base = _bases.find((b) => b.name === baseSel.value);
    if (!base) { zfsBaseFields.style.display = 'none'; sfsBaseFields.style.display = 'none'; return; }

    const isZfs = base.type === 'zfs';
    zfsBaseFields.style.display = isZfs ? '' : 'none';
    sfsBaseFields.style.display = isZfs ? 'none' : '';

    if (isZfs) {
      snapSel.innerHTML = '<option value="">' + t('common.pleaseSelect') + '</option>' +
        (base.snapshots || []).map((s) => {
          const short = s.includes('@') ? s.split('@').pop() : s;
          return `<option value="${escAttr(s)}">${esc(short)}</option>`;
        }).join('');
      const parent = base.source_path.includes('/')
        ? base.source_path.substring(0, base.source_path.lastIndexOf('/'))
        : base.source_path;
      datasetInput.placeholder = `${parent}/${nameInput.value || 'jailname'}`;
    }
  });

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(form);
    const name = fd.get('name');
    const locVal = fd.get('location_type');

    const result = {
      name,
      hostname: fd.get('hostname') || null,
      location_type: locVal,
      interface: fd.get('interface') || null,
      ip4: fd.get('ip4') || null,
      ip6: fd.get('ip6') || null,
    };

    if (locVal === 'directory') {
      result.path = fd.get('dir_path');
    } else if (locVal === 'base') {
      result.base_name = fd.get('base_name');
      const base = _bases.find((b) => b.name === result.base_name);
      if (base && base.type === 'zfs') {
        result.snapshot = fd.get('snapshot');
        result.target_dataset = fd.get('target_dataset') || null;
        result.path = fd.get('base_path') || null;
      } else if (base && base.type === 'sharedfs') {
        result.path = fd.get('sfs_path') || null;
      }
    }

    submitBtn.disabled = true;
    submitBtn.classList.add('btn-loading');
    try {
      await api.post('/api/jails/create', result);
      toast(t('jails.jailCreated'));
      location.hash = '#/jails/running';
    } catch (e) {
      await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
      submitBtn.disabled = false;
      submitBtn.classList.remove('btn-loading');
    }
  });

  setTimeout(() => nameInput.focus(), 50);
}

// ===== Jail detail page =====

export async function renderJailDetail(app, hashPath) {
  const name = hashPath.replace(/^\/jails\/detail\//, '');

  renderLayout(app, '/jails/running', `
    <div class="page-header">
      <div class="flex">
        <a href="#/jails/running" class="btn-secondary btn-sm">${t('common.navBack')}</a>
        <h1>${esc(name)}</h1>
      </div>
      <p>${t('jails.detailSubtitle')}</p>
    </div>
    <div id="jail-detail"><div class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>
  `);

  const el = document.getElementById('jail-detail');

  // Single API call: top-level is config (from jail.conf),
  // runtime is null when stopped.
  let d;
  try {
    d = await api.get(`/api/jails/${encodeURIComponent(name)}`);
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
    return;
  }

  const rt = d.runtime;
  const running = d.jid > 0;
  const p = d.params || {};

  // Merge: conf params as base, libjail params overlay.
  const merged = { ...p };
  if (rt) {
    for (const [k, v] of Object.entries(rt.params || {})) {
      merged[k] = v;
    }
  }

  const ip4Addr = rt?.ip4_addr?.length
    ? rt.ip4_addr
    : (merged['ip4.addr'] ? merged['ip4.addr'].split(',').map(s => s.trim()) : []);
  const ip6Addr = rt?.ip6_addr?.length
    ? rt.ip6_addr
    : (merged['ip6.addr'] ? merged['ip6.addr'].split(',').map(s => s.trim()) : []);

  const jid = d.jid;
  const state = running ? (rt?.state || 'running') : 'stopped';
  const persist = merged.persist === 'true';

  const allowEntries = Object.entries(merged)
    .filter(([k]) => k.startsWith('allow.'))
    .sort(([a], [b]) => a.localeCompare(b));

  const otherEntries = Object.entries(merged)
    .filter(([k]) => !k.startsWith('allow.'))
    .sort(([a], [b]) => a.localeCompare(b));

  el.innerHTML = `
    <div class="card">
      <div class="flex" style="flex-wrap:wrap;gap:16px;align-items:center;">
        <div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">JID</span><strong class="mono">${jid || '—'}</strong></div>
        <div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">${t('common.status')}</span>${stateBadge(state)}</div>
        <div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">persist</span>${boolBadge(persist)}</div>
        ${merged.parent ? `<div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">${t('jails.parent')}</span><strong class="mono">${merged.parent}</strong></div>` : ''}
      </div>
    </div>

    <div class="card">
      <div class="card-title">${t('common.network')}</div>
      <table class="kv-table">
        ${kvRow('interface', merged.interface || '—')}
        ${kvRow('ip4', merged.ip4 || '—')}
        ${kvRow('ip4.addr', ip4Addr.length ? ip4Addr.join(', ') : '—')}
        ${kvRow('ip6', merged.ip6 || '—')}
        ${kvRow('ip6.addr', ip6Addr.length ? ip6Addr.join(', ') : '—')}
        ${kvRow('vnet', merged.vnet || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.hostInfo')}</div>
      <table class="kv-table">
        ${kvRow('host.hostname', merged['host.hostname'] || d.name || '—')}
        ${kvRow('host.domainname', merged['host.domainname'] || '—')}
        ${kvRow('host.hostuuid', merged['host.hostuuid'] || '—')}
        ${kvRow('host.hostid', merged['host.hostid'] || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.security')}</div>
      <table class="kv-table">
        ${kvRow('securelevel', merged.securelevel || '—')}
        ${kvRow('enforce_statfs', merged.enforce_statfs || '—')}
        ${kvRow('devfs_ruleset', merged.devfs_ruleset || '—')}
        ${kvRow('children.max', merged['children.max'] || '—')}
        ${kvRow('children.cur', merged['children.cur'] || '—')}
      </table>
    </div>

    ${rt ? `
    <div class="card">
      <div class="card-title">${t('jails.runtimeInfo')}</div>
      <table class="kv-table">
        ${kvRow('jid', jid)}
        ${kvRow('osrelease', merged.osrelease || '—')}
        ${kvRow('osreldate', merged.osreldate || '—')}
        ${kvRow('cpuset.id', merged['cpuset.id'] || '—')}
        ${kvRow('ip4.saddrsel', merged['ip4.saddrsel'] || '—')}
        ${kvRow('ip6.saddrsel', merged['ip6.saddrsel'] || '—')}
        ${kvRow('dying', merged.dying || 'false')}
      </table>
    </div>` : ''}

    <div class="card">
      <div class="card-title">${t('jails.system')}</div>
      <table class="kv-table">
        ${kvRow('path', merged.path || '—')}
        ${kvRow('exec.start', merged['exec.start'] || '—')}
        ${kvRow('exec.stop', merged['exec.stop'] || '—')}
        ${kvRow('mount.fstab', merged['mount.fstab'] || '—')}
        ${kvRow('mount.devfs', merged['mount.devfs'] || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.permissions')}</div>
      <div class="perm-grid">
        ${allowEntries.map(([k, v]) => permBadge(k.replace(/^allow\./, ''), v === 'true' || v === '1')).join('')}
      </div>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.allParams')}</div>
      <table class="kv-table">
        ${otherEntries.map(([k, v]) => kvRow(k, v || '—')).join('')}
      </table>
    </div>`;
}

// ===== Base systems page =====

export async function renderJailBases(app) {
  renderLayout(app, '/jails/bases', `
    <div class="page-header">
      <h1>${t('jails.basesTitle')}</h1>
      <p>${t('jails.basesSubtitle')}</p>
    </div>
    <div class="toolbar">
      <span id="bases-count" class="text-dim"></span>
      <div class="flex">
        <button onclick="window.__fwpBaseCreate()"><i class="fa-solid fa-plus"></i> ${t('jails.createBase')}</button>
        <button onclick="window.__fwpBasesReload()"><i class="fa-solid fa-rotate-right"></i> ${t('common.refresh')}</button>
      </div>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>${t('common.name')}</th>
          <th>${t('jails.sourcePath')}</th>
          <th>${t('common.type')}</th>
          <th>${t('jails.snapshots')}</th>
          <th>${t('common.actions')}</th>
        </tr></thead>
        <tbody id="bases-tbody">
          <tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);
  await loadBases();
}

async function loadBases() {
  const tbody = document.getElementById('bases-tbody');
  const countEl = document.getElementById('bases-count');
  try {
    const bases = await api.get('/api/jails/bases');
    if (countEl) countEl.textContent = t('jails.basesCount', { n: bases.length });
    if (!bases.length) {
      tbody.innerHTML = `<tr><td colspan="5" class="empty">${t('jails.noBases')}</td></tr>`;
      return;
    }
    tbody.innerHTML = bases.map((b) => `
      <tr>
        <td class="mono"><strong>${esc(b.name)}</strong></td>
        <td class="mono text-dim">${esc(b.source_path)}</td>
        <td>${typeBadge(b)}</td>
        <td class="mono text-dim">${b.snapshots && b.snapshots.length ? b.snapshots.length : '—'}</td>
        <td>
          <div class="btn-group">
            ${b.type === 'zfs' ? `<button class="btn-secondary btn-sm" onclick="window.__fwpBaseEdit('${escAttr(b.name)}')">${t('common.edit')}</button>` : ''}
            <button class="btn-secondary btn-sm" onclick="window.__fwpBaseDelete('${escAttr(b.name)}')">${t('common.delete')}</button>
          </div>
        </td>
      </tr>`).join('');
  } catch (err) {
    if (countEl) countEl.textContent = '';
    tbody.innerHTML = `<tr><td colspan="5" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
  }
}

window.__fwpBasesReload = () => {
  const tbody = document.getElementById('bases-tbody');
  if (tbody) tbody.innerHTML = `<tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>`;
  loadBases();
};

window.__fwpBaseCreate = async () => {
  await createBaseModal(async (result) => {
    await api.post('/api/jails/bases', result);
    toast(t('jails.baseCreated'));
    await loadBases();
  });
};

window.__fwpBaseDelete = async (name) => {
  const ok = await confirmDialog(
    t('common.delete'),
    t('jails.deleteBaseConfirm', { name }),
  );
  if (!ok) return;
  try {
    await api.del(`/api/jails/bases/${encodeURIComponent(name)}`);
    toast(t('jails.baseDeleted'));
    await loadBases();
  } catch (e) {
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
};

window.__fwpBaseEdit = async (name) => {
  let base = null;
  try {
    const bases = await api.get('/api/jails/bases');
    base = bases.find((b) => b.name === name);
  } catch (e) {
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  if (!base) return;

  await editSnapshotsModal(base, async (result) => {
    await api.put(`/api/jails/bases/${encodeURIComponent(name)}`, result);
    toast(t('jails.snapshotsUpdated'));
    await loadBases();
  });
};

function editSnapshotsModal(base, onSubmit) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    const currentSnaps = new Set(base.snapshots || []);

    overlay.innerHTML = `
      <div class="modal" style="max-width:520px;position:relative;">
        <h3>${t('jails.editSnapshots')} — ${esc(base.name)}</h3>
        <p class="text-dim" style="margin-bottom:12px;">${esc(base.source_path)}</p>
        <div class="field">
          <label>${t('jails.selectSnapshots')} <span style="color:var(--danger)">*</span></label>
          <div id="edit-snap-list" style="max-height:200px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);padding:8px;">
            <span class="text-dim">${t('common.loading')}</span>
          </div>
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
          <button type="button" id="edit-snap-save">${t('common.save')}</button>
        </div>
      </div>`;

    document.body.appendChild(overlay);

    const snapList = overlay.querySelector('#edit-snap-list');

    // Load all snapshots for this dataset.
    api.get(`/api/jails/bases/snapshots?name=${encodeURIComponent(base.source_path)}`)
      .then((allSnaps) => {
        if (!allSnaps.length) {
          snapList.innerHTML = `<span class="text-dim">${t('jails.noSnapshots')}</span>`;
          return;
        }
        snapList.innerHTML = allSnaps.map((s) => {
          const short = s.includes('@') ? s.split('@').pop() : s;
          const checked = currentSnaps.has(s) ? 'checked' : '';
          return `<label style="display:flex;align-items:center;gap:6px;padding:3px 0;font-size:13px;cursor:pointer;">
            <input type="checkbox" value="${escAttr(s)}" ${checked} /> ${esc(short)} <span class="text-dim" style="font-size:11px;">${esc(s)}</span></label>`;
        }).join('');
      })
      .catch((e) => {
        snapList.innerHTML = `<span class="text-dim">${esc(e.message || '')}</span>`;
      });

    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'cancel') { overlay.remove(); resolve(null); }
    });

    overlay.querySelector('#edit-snap-save').addEventListener('click', async () => {
      const snaps = [...snapList.querySelectorAll('input[type=checkbox]:checked')].map((cb) => cb.value);
      if (!snaps.length) { await alertDialog(t('common.operationFailed'), t('jails.noSnapshotsSelected')); return; }
      submitModal(overlay, onSubmit, { snapshots: snaps });
    });
  });
}

function renderTypeDesc(container, type) {
  if (!type) { container.innerHTML = ''; return; }
  const desc = type === 'zfs' ? t('jails.zfsTypeDesc') : t('jails.sharedfsTypeDesc');
  const pros = type === 'zfs' ? t('jails.zfsTypePros') : t('jails.sharedfsTypePros');
  const cons = type === 'zfs' ? t('jails.zfsTypeCons') : t('jails.sharedfsTypeCons');
  container.innerHTML = `
    <div class="type-desc-box">
      <p class="type-desc-text">${esc(desc)}</p>
      <div class="type-desc-row">
        <i class="fa-solid fa-circle-check" style="color:var(--success);"></i>
        <span>${esc(pros)}</span>
      </div>
      <div class="type-desc-row">
        <i class="fa-solid fa-circle-xmark" style="color:var(--danger);"></i>
        <span>${esc(cons)}</span>
      </div>
    </div>`;
}

/// Create base system modal — three creation methods with dynamic fields.
function createBaseModal(onSubmit) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    overlay.innerHTML = `
      <div class="modal" style="max-width:600px;">
        <h3>${t('jails.createBase')}</h3>
        <form id="create-base-form">
          <div class="field">
            <label>${t('common.name')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="name" id="cb-name" required placeholder="freebsd-15.1" />
          </div>
          <div class="field">
            <label>${t('jails.creationMethod')} <span style="color:var(--danger)">*</span></label>
            <select name="method" id="cb-method" required>
              <option value="">${t('common.pleaseSelect')}</option>
              <option value="import">${t('jails.methodImport')}</option>
              <option value="from-txz">${t('jails.methodFromTxz')}</option>
              <option value="download">${t('jails.methodDownload')}</option>
            </select>
            <div id="cb-method-desc"></div>
          </div>

          <!-- from-txz: base.txz path -->
          <div id="cb-txz-path-field" style="display:none;">
            <div class="field">
              <label>${t('jails.baseTxzFile')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="txz_path" id="cb-txz-path" placeholder="/path/to/base.txz" />
            </div>
          </div>

          <!-- download: mirror + version (quick-fill) → download URL -->
          <div id="cb-download-fields" style="display:none;">
            <div style="display:flex;gap:12px;">
              <div class="field" style="flex:1;">
                <label>${t('jails.mirror')}</label>
                <select name="mirror" id="cb-mirror"></select>
              </div>
              <div class="field" style="flex:1;">
                <label>${t('jails.version')}</label>
                <input type="text" name="version" id="cb-version" list="cb-version-list" placeholder="" />
                <datalist id="cb-version-list">
                  <option value="15.0-CURRENT">
                  <option value="14.2-RELEASE">
                  <option value="14.1-RELEASE">
                  <option value="13.4-RELEASE">
                  <option value="13.3-RELEASE">
                </datalist>
              </div>
            </div>
            <div class="field">
              <label>${t('jails.downloadUrl')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="download_url" id="cb-download-url" placeholder="${t('jails.downloadUrlPh')}" />
            </div>
          </div>

          <div class="field">
            <label>${t('common.type')} <span style="color:var(--danger)">*</span></label>
            <select name="type" id="cb-type" required>
              <option value="">${t('common.pleaseSelect')}</option>
              <option value="zfs">ZFS ${t('jails.dataset')}</option>
              <option value="sharedfs">SharedFS</option>
            </select>
            <div id="cb-type-desc"></div>
          </div>

          <!-- import + ZFS fields -->
          <div id="cb-import-zfs" style="display:none;">
            <div class="field">
              <label>${t('jails.zfsDataset')} <span style="color:var(--danger)">*</span></label>
              <select name="import_dataset" id="cb-import-dataset">
                <option value="">${t('common.pleaseSelect')}</option>
              </select>
            </div>
            <div class="field" id="cb-import-snap-field" style="display:none;">
              <label>${t('jails.selectSnapshots')} <span style="color:var(--danger)">*</span></label>
              <div id="cb-import-snap-list" style="max-height:160px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);padding:8px;"></div>
            </div>
          </div>

          <!-- import + SharedFS fields -->
          <div id="cb-import-sfs" style="display:none;">
            <div class="field">
              <label>${t('jails.sharedfsDir')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="import_sharedfs" placeholder="/usr/jails/sharedfs" />
            </div>
            <div class="field">
              <label>${t('jails.templateDir')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="import_template" placeholder="/usr/jails/template" />
            </div>
          </div>

          <!-- from-txz / download + ZFS fields -->
          <div id="cb-txz-zfs" style="display:none;">
            <div class="field">
              <label>${t('jails.newDataset')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="dataset" id="cb-dataset" placeholder="zroot/jails/bases/freebsd-15.1" />
            </div>
            <div class="field">
              <label>${t('jails.snapshotName')}</label>
              <input type="text" name="snapshot_name" placeholder="${t('jails.snapshotNamePh')}" />
            </div>
          </div>

          <!-- from-txz / download + SharedFS fields -->
          <div id="cb-txz-sfs" style="display:none;">
            <div class="field">
              <label>${t('jails.newSharedfsDir')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="new_sharedfs" placeholder="/usr/jails/sharedfs/freebsd-15.1" />
            </div>
            <div class="field">
              <label>${t('jails.newTemplateDir')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="new_template" placeholder="/usr/jails/template/freebsd-15.1" />
            </div>
          </div>

          <div class="modal-actions">
            <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
            <button type="submit" id="cb-submit">${t('common.confirm')}</button>
          </div>
        </form>
      </div>`;

    document.body.appendChild(overlay);

    const methodSel = overlay.querySelector('#cb-method');
    const methodDesc = overlay.querySelector('#cb-method-desc');
    const typeSel = overlay.querySelector('#cb-type');
    const typeDesc = overlay.querySelector('#cb-type-desc');
    const importZfs = overlay.querySelector('#cb-import-zfs');
    const importSfs = overlay.querySelector('#cb-import-sfs');
    const txzPathField = overlay.querySelector('#cb-txz-path-field');
    const downloadFields = overlay.querySelector('#cb-download-fields');
    const txzZfs = overlay.querySelector('#cb-txz-zfs');
    const txzSfs = overlay.querySelector('#cb-txz-sfs');
    const datasetSel = overlay.querySelector('#cb-import-dataset');
    const snapField = overlay.querySelector('#cb-import-snap-field');
    const snapList = overlay.querySelector('#cb-import-snap-list');
    const nameInput = overlay.querySelector('#cb-name');
    const datasetInput = overlay.querySelector('#cb-dataset');
    const newSfsInput = overlay.querySelector('input[name="new_sharedfs"]');
    const newTplInput = overlay.querySelector('input[name="new_template"]');
    const submitBtn = overlay.querySelector('#cb-submit');

    // Load ZFS datasets for the import dropdown.
    let _datasets = [];
    api.get('/api/zfs/datasets').then((tree) => {
      _datasets = flattenDatasets(tree);
      datasetSel.innerHTML = `<option value="">${t('common.pleaseSelect')}</option>` +
        _datasets.map((d) => `<option value="${escAttr(d)}">${esc(d)}</option>`).join('');
    }).catch(() => {});

    // Load mirrors for the download dropdown.
    const mirrorSel = overlay.querySelector('#cb-mirror');
    const versionInput = overlay.querySelector('#cb-version');
    const downloadUrlInput = overlay.querySelector('#cb-download-url');
    const _systemArch = navigator.userAgent.includes('aarch64') ? 'arm64' : 'amd64';

    // Load mirrors for the quick-fill dropdown.
    api.get('/api/jails/bases/mirrors').then((mirrors) => {
      mirrorSel.innerHTML = mirrors.map((m) =>
        `<option value="${escAttr(m.url)}">${esc(m.name)}</option>`
      ).join('');
    }).catch(() => {
      mirrorSel.innerHTML = '<option value="https://download.freebsd.org">Official</option>';
    });

    // Pre-fill version with current system version (kern.osrelease).
    api.get('/api/system/info').then((info) => {
      const osRel = info.os_release || '';
      if (osRel) {
        versionInput.value = osRel;
        versionInput.placeholder = osRel;
      }
      updateDownloadUrl();
    }).catch(() => {});

    // Auto-compute download URL from mirror + version.
    function updateDownloadUrl() {
      const mirror = mirrorSel.value;
      const version = versionInput.value.trim();
      if (!mirror || !version) return;
      const branch = version.includes('RELEASE') ? 'releases' : 'snapshots';
      downloadUrlInput.value = `${mirror}/${branch}/${_systemArch}/${version}/base.txz`;
    }

    mirrorSel.addEventListener('change', updateDownloadUrl);
    versionInput.addEventListener('input', updateDownloadUrl);

    function renderMethodDesc(method) {
      if (!method) { methodDesc.innerHTML = ''; return; }
      const descs = {
        'import': t('jails.methodImportDesc'),
        'from-txz': t('jails.methodFromTxzDesc'),
        'download': t('jails.methodDownloadDesc'),
      };
      methodDesc.innerHTML = `<div class="type-desc-box"><p class="type-desc-text">${esc(descs[method] || '')}</p></div>`;
    }

    function updateFields() {
      const method = methodSel.value;
      const type = typeSel.value;
      const isImport = method === 'import';
      const isFromTxz = method === 'from-txz';
      const isDownload = method === 'download';
      const needsTxzPath = isFromTxz;
      const needsDownload = isDownload;
      const needsTxzCreate = isFromTxz || isDownload;

      // Show/hide import fields.
      importZfs.style.display = (isImport && type === 'zfs') ? '' : 'none';
      importSfs.style.display = (isImport && type === 'sharedfs') ? '' : 'none';

      // Show/hide txz path.
      txzPathField.style.display = needsTxzPath ? '' : 'none';

      // Show/hide download fields.
      downloadFields.style.display = needsDownload ? '' : 'none';

      // Show/hide txz-create fields (for from-txz and download).
      txzZfs.style.display = (needsTxzCreate && type === 'zfs') ? '' : 'none';
      txzSfs.style.display = (needsTxzCreate && type === 'sharedfs') ? '' : 'none';

      // Update defaults based on name.
      updateDefaults();
    }

    function updateDefaults() {
      const name = nameInput.value.trim();
      if (datasetInput && !datasetInput.value && name) {
        datasetInput.placeholder = `zroot/jails/bases/${name}`;
      }
      if (newSfsInput && !newSfsInput.value && name) {
        newSfsInput.placeholder = `/usr/jails/sharedfs/${name}`;
      }
      if (newTplInput && !newTplInput.value && name) {
        newTplInput.placeholder = `/usr/jails/template/${name}`;
      }
    }

    methodSel.addEventListener('change', () => {
      renderMethodDesc(methodSel.value);
      updateFields();
    });
    typeSel.addEventListener('change', () => {
      renderTypeDesc(typeDesc, typeSel.value);
      updateFields();
    });
    nameInput.addEventListener('input', updateDefaults);

    // Dataset change → load snapshots (import method only).
    datasetSel.addEventListener('change', async () => {
      const ds = datasetSel.value;
      if (!ds) { snapField.style.display = 'none'; return; }
      snapField.style.display = '';
      snapList.innerHTML = `<span class="text-dim">${t('common.loading')}</span>`;
      try {
        const snaps = await api.get(`/api/jails/bases/snapshots?name=${encodeURIComponent(ds)}`);
        if (!snaps.length) {
          snapList.innerHTML = `<span class="text-dim">${t('jails.noSnapshots')}</span>`;
          return;
        }
        snapList.innerHTML = snaps.map((s) => {
          const short = s.includes('@') ? s.split('@').pop() : s;
          return `<label style="display:flex;align-items:center;gap:6px;padding:3px 0;font-size:13px;cursor:pointer;">
            <input type="checkbox" value="${escAttr(s)}" /> ${esc(short)}</label>`;
        }).join('');
      } catch (e) {
        snapList.innerHTML = `<span class="text-dim">${esc(e.message || '')}</span>`;
      }
    });

    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'cancel') { overlay.remove(); resolve(null); }
    });

    overlay.querySelector('#create-base-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const fd = new FormData(e.target);
      const method = fd.get('method');
      const type = fd.get('type');
      const name = fd.get('name');
      let result = { name, method, type };

      if (method === 'import') {
        if (type === 'zfs') {
          const dataset = fd.get('import_dataset');
          const snaps = [...snapList.querySelectorAll('input[type=checkbox]:checked')].map((cb) => cb.value);
          if (!dataset || !snaps.length) return;
          result.source_path = dataset;
          result.snapshots = snaps;
        } else {
          const sfs = fd.get('import_sharedfs');
          const tpl = fd.get('import_template');
          if (!sfs || !tpl) return;
          result.source_path = tpl;
          result.sharedfs_path = sfs;
        }
      } else if (method === 'from-txz') {
        result.txz_path = fd.get('txz_path');
        if (!result.txz_path) return;
        if (type === 'zfs') {
          result.dataset = fd.get('dataset');
          result.snapshot_name = fd.get('snapshot_name') || null;
          if (!result.dataset) return;
        } else {
          result.sharedfs_path = fd.get('new_sharedfs');
          result.template_path = fd.get('new_template');
          if (!result.sharedfs_path || !result.template_path) return;
        }
      } else if (method === 'download') {
        result.download_url = fd.get('download_url');
        if (!result.download_url) return;
        if (type === 'zfs') {
          result.dataset = fd.get('dataset');
          result.snapshot_name = fd.get('snapshot_name') || null;
          if (!result.dataset) return;
        } else {
          result.sharedfs_path = fd.get('new_sharedfs');
          result.template_path = fd.get('new_template');
          if (!result.sharedfs_path || !result.template_path) return;
        }
      }
      if (result) submitModal(overlay, onSubmit, result);
    });

    setTimeout(() => { const f = overlay.querySelector('input, select'); if (f) f.focus(); }, 50);
  });
}

/// Flatten the ZFS dataset tree into a flat list of names.
function flattenDatasets(tree) {
  const result = [];
  function walk(nodes) {
    for (const n of nodes) {
      result.push(n.name);
      if (n.children) walk(n.children);
    }
  }
  walk(tree);
  return result;
}

// ===== Shared helpers =====

function formatIpStr(ip4, ip6) {
  const parts = [];
  if (ip4 && ip4 !== 'inherit' && ip4 !== 'disable') parts.push(ip4);
  if (ip6 && ip6 !== 'inherit' && ip6 !== 'disable') parts.push(ip6);
  if (ip4 === 'inherit' || ip6 === 'inherit') parts.push('inherit');
  if (!parts.length) return '<span class="text-dim">—</span>';
  return parts.map((ip) => `<span class="badge badge-dim">${esc(ip)}</span>`).join(' ');
}

function stateBadge(state) {
  if (state === 'dying') return `<span class="badge badge-warn">${t('jails.dying')}</span>`;
  if (state === 'stopped') return `<span class="badge badge-dim">${t('jails.stopped')}</span>`;
  if (state === 'running') return `<span class="badge badge-success">${t('jails.running')}</span>`;
  if (state === 'starting') return `<span class="badge badge-warn"><span class="spinner" style="width:11px;height:11px;border-width:1.5px;margin-right:4px;vertical-align:-1px;"></span>${t('jails.starting')}</span>`;
  if (state === 'stopping') return `<span class="badge badge-warn"><span class="spinner" style="width:11px;height:11px;border-width:1.5px;margin-right:4px;vertical-align:-1px;"></span>${t('jails.stopping')}</span>`;
  return `<span class="badge badge-dim">${t('common.unknown')}</span>`;
}

function boolBadge(val) {
  return val
    ? `<span class="badge badge-success">${t('common.enabled')}</span>`
    : `<span class="badge badge-dim">${t('common.disabled')}</span>`;
}

function permBadge(name, allowed) {
  const cls = allowed ? 'badge-success' : 'badge-dim';
  return `<span class="badge ${cls}">${esc(name)}</span>`;
}

function kvRow(key, val) {
  return `<tr><td class="mono text-dim">${esc(key)}</td><td class="mono">${esc(val)}</td></tr>`;
}

function typeBadge(b) {
  if (b.type === 'sharedfs') return `<span class="badge badge-warn">SharedFS</span>`;
  return `<span class="badge badge-success">ZFS</span>`;
}

window.__fwpJailReload = () => {
  const tbody = document.getElementById('jail-tbody');
  if (tbody) tbody.innerHTML = `<tr><td colspan="7" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>`;
  loadJails();
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
