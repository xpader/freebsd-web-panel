// Jails — list running jail containers, view details, and manage base systems.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { confirmDialog } from '../ui/confirm.js';
import { toast } from '../ui/toast.js';
import { t } from '../i18n/index.js';

/// Show a loading overlay on top of a modal element, call the async onSubmit,
/// close on success or show error toast on failure.
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
    toast(e.message || t('common.operationFailed'), 'error');
  }
}

// ===== Jail list (with Running / All tabs) =====

let _jailTab = 'running';

export async function renderJailsRunning(app) {
  renderLayout(app, '/jails/running', `
    <div class="page-header">
      <h1>${t('jails.title')}</h1>
      <p>${t('jails.subtitle')}</p>
    </div>
    <div class="toolbar">
      <div class="filter-group" id="jail-tab-group">
        <button class="filter-btn active" data-val="running">${t('jails.tabRunning')}</button>
        <button class="filter-btn" data-val="all">${t('jails.tabAll')}</button>
      </div>
      <span id="jail-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpJailReload()">${t('common.refresh')}</button>
      <button onclick="window.__fwpJailCreate()">${t('jails.create')}</button>
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

  _jailTab = 'running';
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
  const url = _jailTab === 'running' ? '/api/jails' : '/api/jails/all';

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
          <td class="mono">${formatIps(j.ip4_addr, j.ip6_addr)}</td>
          <td>${stateBadge(j.state)}</td>
          <td>${actionButtons(j.name, true)}</td>
        </tr>`).join('');
    } else {
      // All jails from jail.conf.
      tbody.innerHTML = data.map((j) => `
        <tr class="row-clickable" onclick="location.hash='#/jails/detail/${escAttr(j.name)}'">
          <td class="mono text-dim">${j.running ? j.jid || '—' : '—'}</td>
          <td class="mono"><strong>${esc(j.name)}</strong></td>
          <td>${esc(j.hostname || '—')}</td>
          <td class="mono text-dim">${esc(j.path || '—')}</td>
          <td class="mono">${formatConfIps(j)}</td>
          <td>${j.running ? stateBadge('running') : `<span class="badge badge-dim">${t('jails.stopped')}</span>`}</td>
          <td>${actionButtons(j.name, j.running)}</td>
        </tr>`).join('');
    }
  } catch (err) {
    if (countEl) countEl.textContent = '';
    tbody.innerHTML = `<tr><td colspan="7" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
  }
}

// ===== Jail actions (start/stop/delete) =====

function actionButtons(name, running) {
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
  try {
    await api.post(`/api/jails/${encodeURIComponent(name)}/${action}`);
    toast(t('jails.actionDone', { name, action: t('jails.' + action) }));
    await loadJails();
  } catch (e) {
    toast(e.message || t('common.operationFailed'), 'error');
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
    toast(e.message || t('common.operationFailed'), 'error');
  }
};

// ===== Create jail =====

window.__fwpJailCreate = async () => {
  let bases = [];
  try { bases = await api.get('/api/jails/bases'); } catch {}

  await createJailModal(bases, async (result) => {
    await api.post('/api/jails/create', result);
    toast(t('jails.jailCreated'));
    _jailTab = 'all';
    document.querySelectorAll('#jail-tab-group .filter-btn').forEach((b) => {
      b.classList.toggle('active', b.dataset.val === 'all');
    });
    await loadJails();
  });
};

function createJailModal(bases, onSubmit) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    const baseOptions = bases.map((b) =>
      `<option value="${escAttr(b.name)}">${esc(b.name)} (${b.type})</option>`
    ).join('');

    overlay.innerHTML = `
      <div class="modal" style="max-width:560px;">
        <h3>${t('jails.create')}</h3>
        <form id="create-jail-form">
          <div class="field">
            <label>${t('jails.jailName')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="name" id="cj-name" required placeholder="web01" />
          </div>
          <div class="field">
            <label>${t('jails.hostname')}</label>
            <input type="text" name="hostname" placeholder="${t('jails.hostnamePh')}" />
          </div>
          <div class="field">
            <label>${t('jails.locationType')} <span style="color:var(--danger)">*</span></label>
            <select name="location_type" id="cj-loc-type" required>
              <option value="">${t('common.pleaseSelect')}</option>
              <option value="directory">${t('jails.locDirectory')}</option>
              <option value="base">${t('jails.locBase')}</option>
            </select>
          </div>

          <div id="cj-dir-fields" style="display:none;">
            <div class="field">
              <label>${t('jails.path')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="dir_path" id="cj-dir-path" placeholder="/jails/web01" />
            </div>
          </div>

          <div id="cj-base-fields" style="display:none;">
            <div class="field">
              <label>${t('jails.selectBase')} <span style="color:var(--danger)">*</span></label>
              <select name="base_name" id="cj-base-name">
                <option value="">${t('common.pleaseSelect')}</option>
                ${baseOptions}
              </select>
            </div>
            <div id="cj-zfs-base-fields" style="display:none;">
              <div class="field">
                <label>${t('jails.cloneSnapshot')} <span style="color:var(--danger)">*</span></label>
                <select name="snapshot" id="cj-snapshot"></select>
              </div>
              <div class="field">
                <label>${t('jails.targetDataset')}</label>
                <input type="text" name="target_dataset" id="cj-dataset" placeholder="${t('jails.datasetDefault')}" />
              </div>
              <div class="field">
                <label>${t('jails.mountPoint')}</label>
                <input type="text" name="base_path" id="cj-mountpoint" placeholder="/jails/web01" />
              </div>
            </div>
            <div id="cj-sfs-base-fields" style="display:none;">
              <div class="field">
                <label>${t('jails.targetLocation')}</label>
                <input type="text" name="sfs_path" id="cj-sfs-path" placeholder="/jails/web01" />
              </div>
            </div>
          </div>

          <div class="field">
            <label>${t('jails.networkInterface')}</label>
            <input type="text" name="interface" placeholder="bge0" />
          </div>
          <div class="field">
            <label>IPv4</label>
            <input type="text" name="ip4" placeholder="${t('jails.ip4Ph')}" />
          </div>
          <div class="field">
            <label>IPv6</label>
            <input type="text" name="ip6" placeholder="${t('jails.ip6Ph')}" />
          </div>

          <div class="modal-actions">
            <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
            <button type="submit">${t('common.confirm')}</button>
          </div>
        </form>
      </div>`;

    document.body.appendChild(overlay);

    const nameInput = overlay.querySelector('#cj-name');
    const locType = overlay.querySelector('#cj-loc-type');
    const dirFields = overlay.querySelector('#cj-dir-fields');
    const baseFields = overlay.querySelector('#cj-base-fields');
    const baseSel = overlay.querySelector('#cj-base-name');
    const zfsBaseFields = overlay.querySelector('#cj-zfs-base-fields');
    const sfsBaseFields = overlay.querySelector('#cj-sfs-base-fields');
    const snapSel = overlay.querySelector('#cj-snapshot');
    const datasetInput = overlay.querySelector('#cj-dataset');
    const mountInput = overlay.querySelector('#cj-mountpoint');
    const sfsPathInput = overlay.querySelector('#cj-sfs-path');

    let _bases = bases;

    // Update default paths when name changes.
    const updateDefaults = () => {
      const name = nameInput.value.trim();
      if (mountInput && !mountInput.value && name) mountInput.placeholder = `/jails/${name}`;
      if (sfsPathInput && !sfsPathInput.value && name) sfsPathInput.placeholder = `/jails/${name}`;
      if (dirFields.style.display !== 'none') {
        const dp = overlay.querySelector('#cj-dir-path');
        if (dp && !dp.value && name) dp.placeholder = `/jails/${name}`;
      }
    };
    nameInput.addEventListener('input', updateDefaults);

    // Location type toggle.
    locType.addEventListener('change', () => {
      const isDir = locType.value === 'directory';
      const isBase = locType.value === 'base';
      dirFields.style.display = isDir ? '' : 'none';
      baseFields.style.display = isBase ? '' : 'none';
      updateDefaults();
    });

    // Base system selection → show ZFS or SharedFS fields.
    baseSel.addEventListener('change', () => {
      const base = _bases.find((b) => b.name === baseSel.value);
      if (!base) { zfsBaseFields.style.display = 'none'; sfsBaseFields.style.display = 'none'; return; }

      const isZfs = base.type === 'zfs';
      zfsBaseFields.style.display = isZfs ? '' : 'none';
      sfsBaseFields.style.display = isZfs ? 'none' : '';

      if (isZfs) {
        // Populate snapshot options.
        snapSel.innerHTML = '<option value="">' + t('common.pleaseSelect') + '</option>' +
          (base.snapshots || []).map((s) => {
            const short = s.includes('@') ? s.split('@').pop() : s;
            return `<option value="${escAttr(s)}">${esc(short)}</option>`;
          }).join('');
        // Default dataset from base source_path parent.
        const parent = base.source_path.includes('/')
          ? base.source_path.substring(0, base.source_path.lastIndexOf('/'))
          : base.source_path;
        datasetInput.placeholder = `${parent}/${nameInput.value || 'jailname'}`;
      }
    });

    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'cancel') { overlay.remove(); resolve(null); }
    });

    overlay.querySelector('#create-jail-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const fd = new FormData(e.target);
      const name = fd.get('name');
      const locType = fd.get('location_type');

      const result = {
        name,
        hostname: fd.get('hostname') || null,
        location_type: locType,
        interface: fd.get('interface') || null,
        ip4: fd.get('ip4') || null,
        ip6: fd.get('ip6') || null,
      };

      if (locType === 'directory') {
        result.path = fd.get('dir_path');
      } else if (locType === 'base') {
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

      submitModal(overlay, onSubmit, result);
    });

    setTimeout(() => nameInput.focus(), 50);
  });
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
  let d;
  try {
    d = await api.get(`/api/jails/${encodeURIComponent(name)}`);
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
    return;
  }

  const allowEntries = Object.entries(d.params)
    .filter(([k]) => k.startsWith('allow.'))
    .sort(([a], [b]) => a.localeCompare(b));

  const otherEntries = Object.entries(d.params)
    .filter(([k]) => !k.startsWith('allow.'))
    .sort(([a], [b]) => a.localeCompare(b));

  el.innerHTML = `
    <div class="stat-grid">
      <div class="card"><div class="card-title">JID</div><div class="card-value sm">${d.jid}</div></div>
      <div class="card"><div class="card-title">${t('common.status')}</div><div class="card-value sm">${stateBadge(d.state)}</div></div>
      <div class="card"><div class="card-title">persist</div><div class="card-value sm">${boolBadge(d.persist)}</div></div>
      <div class="card"><div class="card-title">${t('jails.parent')}</div><div class="card-value sm">${d.params.parent || '0'}</div></div>
    </div>

    <div class="card">
      <div class="card-title">${t('common.network')}</div>
      <table class="kv-table">
        ${kvRow('ip4.addr', d.ip4_addr.length ? d.ip4_addr.join(', ') : '—')}
        ${kvRow('ip4.saddrsel', d.params['ip4.saddrsel'] || '—')}
        ${kvRow('ip6.addr', d.ip6_addr.length ? d.ip6_addr.join(', ') : '—')}
        ${kvRow('ip6.saddrsel', d.params['ip6.saddrsel'] || '—')}
        ${kvRow('vnet', d.params.vnet || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.hostInfo')}</div>
      <table class="kv-table">
        ${kvRow('host.hostname', d.hostname || '—')}
        ${kvRow('host.domainname', d.params['host.domainname'] || '—')}
        ${kvRow('host.hostuuid', d.params['host.hostuuid'] || '—')}
        ${kvRow('host.hostid', d.params['host.hostid'] || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.security')}</div>
      <table class="kv-table">
        ${kvRow('securelevel', d.params.securelevel || '—')}
        ${kvRow('enforce_statfs', d.params.enforce_statfs || '—')}
        ${kvRow('devfs_ruleset', d.params.devfs_ruleset || '—')}
        ${kvRow('children.max', d.params['children.max'] || '—')}
        ${kvRow('children.cur', d.params['children.cur'] || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.system')}</div>
      <table class="kv-table">
        ${kvRow('osrelease', d.params.osrelease || '—')}
        ${kvRow('osreldate', d.params.osreldate || '—')}
        ${kvRow('cpuset.id', d.params['cpuset.id'] || '—')}
        ${kvRow('path', d.path || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.permissions')}</div>
      <div class="perm-grid">
        ${allowEntries.map(([k, v]) => permBadge(k.replace(/^allow\./, ''), v === 'true')).join('')}
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
      <div></div>
      <button onclick="window.__fwpBasesReload()">${t('common.refresh')}</button>
      <button onclick="window.__fwpBaseImport()">${t('jails.importBase')}</button>
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
            <button class="btn-secondary btn-sm" onclick="window.__fwpBaseImage('${escAttr(b.name)}')">${t('jails.createImage')}</button>
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

window.__fwpBaseImport = async () => {
  await importBaseModal(async (result) => {
    await api.post('/api/jails/bases', result);
    toast(t('jails.baseImported'));
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
    toast(e.message || t('common.operationFailed'), 'error');
  }
};

window.__fwpBaseEdit = async (name) => {
  let base = null;
  try {
    const bases = await api.get('/api/jails/bases');
    base = bases.find((b) => b.name === name);
  } catch (e) {
    toast(e.message || t('common.operationFailed'), 'error');
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

    overlay.querySelector('#edit-snap-save').addEventListener('click', () => {
      const snaps = [...snapList.querySelectorAll('input[type=checkbox]:checked')].map((cb) => cb.value);
      if (!snaps.length) { toast(t('jails.noSnapshotsSelected'), 'error'); return; }
      submitModal(overlay, onSubmit, { snapshots: snaps });
    });
  });
}

window.__fwpBaseImage = async (name) => {
  let base = null;
  try {
    const bases = await api.get('/api/jails/bases');
    base = bases.find((b) => b.name === name);
  } catch (e) {
    toast(e.message || t('common.operationFailed'), 'error');
    return;
  }
  if (!base) return;

  await createImageModal(base, async (result) => {
    await api.post(`/api/jails/bases/${encodeURIComponent(name)}/image`, result);
    toast(t('jails.imageCreated'));
  });
};

/// Import base system modal — type selector with dynamic fields.
function importBaseModal(onSubmit) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    overlay.innerHTML = `
      <div class="modal" style="max-width:560px;">
        <h3>${t('jails.importBase')}</h3>
        <form id="import-form">
          <div class="field">
            <label>${t('common.name')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="name" required placeholder="freebsd-15.1" />
          </div>
          <div class="field">
            <label>${t('jails.baseType')} <span style="color:var(--danger)">*</span></label>
            <select name="type" id="import-type" required>
              <option value="">${t('common.pleaseSelect')}</option>
              <option value="zfs">ZFS ${t('jails.dataset')}</option>
              <option value="sharedfs">SharedFS</option>
            </select>
          </div>
          <div id="zfs-import-fields" style="display:none;">
            <div class="field">
              <label>${t('jails.zfsDataset')} <span style="color:var(--danger)">*</span></label>
              <select name="zfs_dataset" id="import-zfs-dataset">
                <option value="">${t('common.pleaseSelect')}</option>
              </select>
            </div>
            <div class="field" id="zfs-snap-field" style="display:none;">
              <label>${t('jails.selectSnapshots')} <span style="color:var(--danger)">*</span></label>
              <div id="zfs-snap-list" style="max-height:160px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);padding:8px;"></div>
            </div>
          </div>
          <div id="sharedfs-import-fields" style="display:none;">
            <div class="field">
              <label>${t('jails.sharedfsDir')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="sharedfs_path" placeholder="/usr/jails/sharedfs" />
            </div>
            <div class="field">
              <label>${t('jails.templateDir')} <span style="color:var(--danger)">*</span></label>
              <input type="text" name="template_path" placeholder="/usr/jails/template" />
            </div>
          </div>
          <div class="modal-actions">
            <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
            <button type="submit">${t('common.confirm')}</button>
          </div>
        </form>
      </div>`;

    document.body.appendChild(overlay);

    const typeSel = overlay.querySelector('#import-type');
    const zfsFields = overlay.querySelector('#zfs-import-fields');
    const sfsFields = overlay.querySelector('#sharedfs-import-fields');
    const datasetSel = overlay.querySelector('#import-zfs-dataset');
    const snapField = overlay.querySelector('#zfs-snap-field');
    const snapList = overlay.querySelector('#zfs-snap-list');

    // Load ZFS datasets for the dropdown.
    let _datasets = [];
    api.get('/api/zfs/datasets').then((tree) => {
      _datasets = flattenDatasets(tree);
      datasetSel.innerHTML = `<option value="">${t('common.pleaseSelect')}</option>` +
        _datasets.map((d) => `<option value="${escAttr(d)}">${esc(d)}</option>`).join('');
    }).catch(() => {});

    // Type toggle.
    typeSel.addEventListener('change', () => {
      const isZfs = typeSel.value === 'zfs';
      const isSfs = typeSel.value === 'sharedfs';
      zfsFields.style.display = isZfs ? '' : 'none';
      sfsFields.style.display = isSfs ? '' : 'none';
    });

    // Dataset change → load snapshots.
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

    overlay.querySelector('#import-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const fd = new FormData(e.target);
      const type = fd.get('type');
      const name = fd.get('name');
      let result;
      if (type === 'zfs') {
        const dataset = fd.get('zfs_dataset');
        const snaps = [...snapList.querySelectorAll('input[type=checkbox]:checked')].map((cb) => cb.value);
        if (!dataset || !snaps.length) return;
        result = { name, type, source_path: dataset, snapshots: snaps };
      } else if (type === 'sharedfs') {
        const sfs = fd.get('sharedfs_path');
        const tpl = fd.get('template_path');
        if (!sfs || !tpl) return;
        result = { name, type: 'sharedfs', source_path: tpl, sharedfs_path: sfs };
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

/// Create image modal — fields depend on base system type.
function createImageModal(base, onSubmit) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    const isZfs = base.type === 'zfs';

    const snapOptions = (base.snapshots || []).map((s) => {
      const shortName = s.includes('@') ? s.split('@').pop() : s;
      return `<option value="${escAttr(s)}">${esc(shortName)}</option>`;
    }).join('');

    overlay.innerHTML = `
      <div class="modal" style="max-width:520px;">
        <h3>${t('jails.createImage')} — ${esc(base.name)}</h3>
        <form id="img-form">
          ${isZfs ? `
          <div class="field">
            <label>${t('jails.cloneSnapshot')} <span style="color:var(--danger)">*</span></label>
            <select name="snapshot" required>
              <option value="">${t('common.pleaseSelect')}</option>
              ${snapOptions}
            </select>
          </div>
          <div class="field">
            <label>${t('jails.targetDataset')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="dataset" required placeholder="zroot/jails/web01" />
          </div>
          <div class="field">
            <label>${t('jails.targetLocation')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="target" required placeholder="/jails/web01" />
          </div>
          ` : `
          <div class="field">
            <label>${t('jails.targetLocation')} <span style="color:var(--danger)">*</span></label>
            <input type="text" name="target" required placeholder="/jails/web01" />
          </div>
          `}
          <input type="hidden" name="method" value="${isZfs ? 'zfs-clone' : 'sharedfs'}" />
          <div class="modal-actions">
            <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
            <button type="submit">${t('common.confirm')}</button>
          </div>
        </form>
      </div>`;

    document.body.appendChild(overlay);

    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'cancel') { overlay.remove(); resolve(null); }
    });

    overlay.querySelector('#img-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const fd = new FormData(e.target);
      const result = {
        method: fd.get('method'),
        target: fd.get('target'),
      };
      if (isZfs) {
        result.snapshot = fd.get('snapshot');
        result.dataset = fd.get('dataset');
      }
      submitModal(overlay, onSubmit, result);
    });

    setTimeout(() => { const f = overlay.querySelector('input, select'); if (f) f.focus(); }, 50);
  });
}

// ===== Shared helpers =====

function formatIps(ip4, ip6) {
  const all = [...(ip4 || []), ...(ip6 || [])];
  if (!all.length) return '<span class="text-dim">—</span>';
  return all.map((ip) => `<span class="badge badge-dim">${esc(ip)}</span>`).join(' ');
}

/// Format IP info from a jail.conf entry (has ip4 / ip4_addr fields).
function formatConfIps(j) {
  if (j.ip4_addr) return `<span class="badge badge-dim">${esc(j.ip4_addr)}</span>`;
  if (j.ip4) return `<span class="badge badge-dim">${esc(j.ip4)}</span>`;
  return '<span class="text-dim">—</span>';
}

function stateBadge(state) {
  if (state === 'dying') return `<span class="badge badge-warn">${t('jails.dying')}</span>`;
  return `<span class="badge badge-success">${t('jails.running')}</span>`;
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
