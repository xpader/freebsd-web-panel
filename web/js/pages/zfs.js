// ZFS management — pools, datasets, snapshots.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { formModal } from '../ui/formModal.js';
import { t, getLocale } from '../i18n/index.js';
// ===== Zpool list page =====

export async function renderZfsPools(app) {
  renderLayout(app, '/zfs/pools', `
    <div class="page-header">
      <h1>Zpool</h1>
      <p>${t('zfs.poolsSubtitle')}</p>
    </div>
    <div id="zfs-pools"><div class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>
  `);
  const el = document.getElementById('zfs-pools');
  try {
    const pools = await api.get('/api/zfs/pools');
    if (!pools.length) { el.innerHTML = `<div class="empty">${t('zfs.noPools')}</div>`; return; }
    el.innerHTML = pools.map(p => poolCard(p)).join('');
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
  }
}

function poolCard(p) {
  const pct = p.capacity_pct;
  const healthCls = p.health === 'ONLINE' ? 'badge-success' : 'badge-danger';
  return `
    <div class="card pool-card" style="cursor:pointer;" onclick="location.hash='#/zfs/pools/${esc(p.name)}'">
      <div class="flex" style="justify-content:space-between;">
        <div>
          <span class="badge ${healthCls}">${esc(p.health)}</span>
          <strong style="font-size:18px;margin-left:8px;">${esc(p.name)}</strong>
        </div>
        <span class="text-dim" style="font-size:13px;">${t('zfs.usedPct', { pct: pct.toFixed(0) })}</span>
      </div>
      <div class="stat-row" style="margin-top:12px;">
        <span>${t('common.capacity')}: <strong>${fmtBytes(p.size)}</strong></span>
        <span>${t('common.used')}: ${fmtBytes(p.allocated)}</span>
        <span>${t('common.free')}: ${fmtBytes(p.free)}</span>
        <span>${t('common.frag')}: ${p.fragmentation_pct.toFixed(0)}%</span>
        <span>${t('common.dedup')}: ${p.dedup.toFixed(2)}x</span>
      </div>
      <div class="bar-wrap" style="margin-top:10px;">
        <div class="bar ${pct > 80 ? 'bar-swap' : 'bar-mem'}" style="width:${pct}%"></div>
      </div>
    </div>`;
}

// ===== Zpool detail page =====

export async function renderZfsPoolDetail(app, hashPath) {
  // Extract pool name from path: /zfs/pools/{name}
  const name = hashPath.replace(/^\/zfs\/pools\//, '');

  renderLayout(app, '/zfs/pools', `
    <div class="page-header">
      <div class="flex">
        <a href="#/zfs/pools" class="btn-secondary btn-sm">${t('common.navBack')}</a>
        <h1 id="pool-title">${t('zfs.poolTitle', { name: esc(name) })}</h1>
      </div>
      <p>${t('zfs.poolDetailSubtitle')}</p>
    </div>
    <div id="pool-detail"><div class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>
  `);

  const el = document.getElementById('pool-detail');
  let info;
  try {
    info = await api.get(`/api/zfs/pools/${name}`);
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
    return;
  }

  const pct = info.capacity_pct || 0;
  const healthCls = healthBadge(info.health);
  const fragCls = info.fragmentation_pct > 70 ? 'badge-danger' : info.fragmentation_pct > 50 ? 'badge-warn' : 'badge-success';

  el.innerHTML = `
    <!-- Summary cards -->
    <div class="stat-grid">
      <div class="card"><div class="card-title">${t('zfs.state')}</div><div class="card-value sm"><span class="badge ${healthCls}">${esc(info.health)}</span></div></div>
      <div class="card"><div class="card-title">${t('zfs.totalSize')}</div><div class="card-value sm">${fmtBytes(info.size)}</div></div>
      <div class="card"><div class="card-title">${t('zfs.allocated')}</div><div class="card-value sm">${fmtBytes(info.allocated)} (${pct.toFixed(0)}%)</div></div>
      <div class="card"><div class="card-title">${t('common.free')}</div><div class="card-value sm">${fmtBytes(info.free)}</div></div>
      <div class="card"><div class="card-title">${t('common.frag')}</div><div class="card-value sm"><span class="badge ${fragCls}">${info.fragmentation_pct.toFixed(0)}%</span></div></div>
      <div class="card"><div class="card-title">${t('common.dedup')}</div><div class="card-value sm">${info.dedup.toFixed(2)}x</div></div>
    </div>

    <!-- Capacity bar -->
    <div class="card">
      <div class="card-title">${t('zfs.capacityUsage')}</div>
      <div class="bar-wrap" style="height:16px;">
        <div class="bar ${pct > 80 ? 'bar-swap' : 'bar-mem'}" style="width:${pct}%"></div>
      </div>
      <div class="text-dim" style="font-size:12px;margin-top:6px;">${fmtBytes(info.allocated)} / ${fmtBytes(info.size)} (${pct.toFixed(1)}%)</div>
    </div>

    <!-- Scrub info -->
    ${info.scan ? `
    <div class="card">
      <div class="card-title">${t('zfs.scrubStatus')}</div>
      <p style="font-size:13px;">${esc(info.scan)}</p>
    </div>` : ''}

    <!-- VDEV / Disk topology -->
    <div class="card">
      <div class="card-title">${t('zfs.vdevTree')}</div>
      ${renderVdevTree(info.vdevs, 0)}
    </div>

    <!-- Errors -->
    ${info.error_text && !info.error_text.includes('No known') ? `
    <div class="card" style="border-color:var(--danger);">
      <div class="card-title" style="color:var(--danger);">${t('zfs.errors')}</div>
      <p style="font-size:13px;color:var(--danger);">${esc(info.error_text)}</p>
    </div>` : ''}

    <!-- Actions -->
    <div class="card">
      <div class="card-title">${t('zfs.maintenance')}</div>
      <div class="flex" style="gap:12px;">
        <button class="btn-secondary" id="btn-scrub">${t('zfs.scrubStart')}</button>
        <button class="btn-secondary" id="btn-scrub-stop">${t('zfs.scrubStop')}</button>
      </div>
      <p class="text-dim" style="font-size:12px;margin-top:10px;">
        ${t('zfs.scrubHint')}
      </p>
    </div>`;

  // Attach action buttons.
  document.getElementById('btn-scrub').onclick = async () => {
    try {
      await api.post(`/api/zfs/pools/${name}/scrub`);
      toast(t('zfs.scrubStarted', { name }));
      renderZfsPoolDetail(document.getElementById('app'), hashPath);
    } catch (e) { toast(e.message || t('common.operationFailed'), 'error'); }
  };
  document.getElementById('btn-scrub-stop').onclick = async () => {
    try {
      await api.post(`/api/zfs/pools/${name}/scrub/stop`);
      toast(t('zfs.scrubStopped', { name }));
      renderZfsPoolDetail(document.getElementById('app'), hashPath);
    } catch (e) { toast(e.message || t('common.operationFailed'), 'error'); }
  };
}

function healthBadge(health) {
  if (health === 'ONLINE') return 'badge-success';
  if (health === 'DEGRADED') return 'badge-warn';
  return 'badge-danger';
}

function renderVdevTree(vdevs, depth) {
  if (!vdevs || !vdevs.length) return `<div class="empty">${t('zfs.noVdev')}</div>`;
  return vdevs.map(v => {
    const isLeaf = !v.children.length;
    const isMirror = v.name.startsWith('mirror');
    const isRaidz = v.name.startsWith('raidz');
    const vdevType = isMirror ? t('zfs.vdevMirror') : isRaidz ? t('zfs.vdevRaidz') : isLeaf ? t('zfs.vdevDisk') : t('zfs.vdevGeneric');
    const stateCls = v.state === 'ONLINE' ? 'badge-success' : v.state === 'DEGRADED' ? 'badge-warn' : 'badge-danger';
    const hasErrors = v.read_errors > 0 || v.write_errors > 0 || v.checksum_errors > 0;
    return `
      <div class="vdev-node" style="margin-left:${depth * 24}px;">
        <div class="vdev-node-row flex">
          <span class="mono" style="font-size:13px;font-weight:${depth === 0 ? '600' : '400'};">${esc(v.name)}</span>
          <span class="badge badge-dim" style="margin:0 8px;font-size:10px;">${vdevType}</span>
          <span class="badge ${stateCls}" style="font-size:10px;">${esc(v.state)}</span>
          ${hasErrors ? `<span class="badge badge-danger" style="font-size:10px;">R:${v.read_errors} W:${v.write_errors} C:${v.checksum_errors}</span>` : ''}
        </div>
        ${v.children.length ? renderVdevTree(v.children, depth + 1) : ''}
      </div>`;
  }).join('');
}

// ===== Dataset management =====

export async function renderZfsDatasets(app) {
  renderLayout(app, '/zfs/datasets', `
    <div class="page-header">
      <h1>${t('zfs.dsTitle')}</h1>
      <p>${t('zfs.dsSubtitle')}</p>
    </div>
    <div class="toolbar">
      <div></div>
      <button onclick="window.__fwpCreateDataset()">${t('zfs.dsCreate')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('common.name')}</th><th>${t('common.type')}</th><th>${t('common.used')}</th><th>${t('zfs.colAvail')}</th><th>${t('zfs.colMountpoint')}</th><th>${t('zfs.colCompression')}</th><th>${t('common.actions')}</th></tr></thead>
        <tbody id="ds-tbody"><tr><td colspan="7" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr></tbody>
      </table>
    </div>
  `);
  await loadDatasets();
}

async function loadDatasets() {
  const tbody = document.getElementById('ds-tbody');
  try {
    const tree = await api.get('/api/zfs/datasets');
    const rows = [];
    function walk(ds, depth) {
      const originHtml = ds.origin
        ? `<div class="text-dim" style="font-size:11px;margin-top:2px;">${t('zfs.clonedFrom')} <span class="mono" style="color:var(--accent);">${esc(ds.origin)}</span></div>`
        : '';
      rows.push(`
        <tr>
          <td class="mono" style="padding-left:${depth * 20 + 12}px;">
            ${depth > 0 ? '└ ' : ''}<strong>${esc(ds.name)}</strong>
            ${originHtml}
          </td>
          <td><span class="badge badge-dim">${esc(ds.typ)}</span></td>
          <td class="mono">${fmtBytes(ds.used)}</td>
          <td class="mono">${fmtBytes(ds.available)}</td>
          <td class="mono">${esc(ds.mountpoint)}</td>
          <td class="mono">${esc(ds.compression)}</td>
          <td>
            <button class="btn-secondary btn-sm" onclick="window.__fwpDsSnap('${esc(ds.name)}')">${t('zfs.dsSnapshot')}</button>
            <button class="btn-secondary btn-sm" onclick="window.__fwpDsProps('${esc(ds.name)}')">${t('zfs.dsProps')}</button>
            ${ds.name.includes('/') ? `<button class="btn-danger btn-sm" onclick="window.__fwpDelDs('${esc(ds.name)}')">${t('common.delete')}</button>` : ''}
          </td>
        </tr>`);
      ds.children.forEach(c => walk(c, depth + 1));
    }
    tree.forEach(ds => walk(ds, 0));
    tbody.innerHTML = rows.join('') || `<tr><td colspan="7" class="empty">${t('zfs.noDatasets')}</td></tr>`;
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="7" class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</td></tr>`;
  }
}

window.__fwpCreateDataset = async () => {
  const result = await formModal(t('zfs.dsCreateTitle'), [
    { key: 'name', label: t('zfs.dsNameLabel'), placeholder: t('zfs.dsNamePlaceholder'), required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/datasets', { name: result.name }).then(() => {
    toast(t('zfs.dsCreated'));
    loadDatasets();
  }).catch(e => toast(e.message || t('zfs.dsCreateFailedShort'), 'error'));
};

window.__fwpDsSnap = async (name) => {
  const result = await formModal(t('zfs.dsCreateSnapTitle', { name }), [
    { key: 'name', label: t('zfs.snapNameLabel'), placeholder: t('zfs.snapNamePlaceholder'), required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/snapshots', { dataset: name, name: result.name }).then(() => {
    toast(t('zfs.snapCreated', { name, snap: result.name }));
  }).catch(e => toast(e.message || t('zfs.snapCreateFailed'), 'error'));
};

window.__fwpDelDs = async (name) => {
  if (!await confirmDialog(t('zfs.dsDeleteTitle'), t('zfs.dsDeleteConfirm', { name }))) return;
  api.del(`/api/zfs/dataset/destroy?name=${encodeURIComponent(name)}`).then(() => {
    toast(t('zfs.dsDeleted'));
    loadDatasets();
  }).catch(e => toast(e.message || t('zfs.dsDeleteFailed'), 'error'));
};

window.__fwpDsProps = async (name) => {
  let props;
  try { props = await api.get(`/api/zfs/dataset/properties?name=${encodeURIComponent(name)}`); }
  catch (e) { toast(e.message || t('zfs.dsPropsFailed'), 'error'); return; }

  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal" style="max-width:600px;">
      <h3>${t('zfs.propsTitle', { name: esc(name) })}</h3>
      <div style="max-height:400px;overflow-y:auto;">
        <table style="font-size:12px;">
          <thead><tr><th>${t('common.name')}</th><th>${t('zfs.colValue')}</th><th>${t('zfs.colSource')}</th></tr></thead>
          <tbody>
            ${props.map(p => `<tr><td class="mono">${esc(p.name)}</td><td class="mono">${esc(p.value)}</td><td class="text-dim mono">${esc(p.source)}</td></tr>`).join('')}
          </tbody>
        </table>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="close">${t('common.close')}</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay || e.target.dataset.act === 'close') overlay.remove();
  });
};

// ===== Snapshot management =====

export async function renderZfsSnapshots(app) {
  renderLayout(app, '/zfs/snapshots', `
    <div class="page-header">
      <h1>${t('zfs.snapTitle')}</h1>
      <p>${t('zfs.snapSubtitle')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="snap-filter" class="search" placeholder="${t('zfs.snapFilter')}" oninput="window.__fwpSnapFilter()" />
      <button onclick="window.__fwpCreateSnap()">${t('zfs.snapCreate')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('zfs.dsLabel')}</th><th>${t('zfs.colSnapshot')}</th><th>${t('common.used')}</th><th>${t('zfs.colRefer')}</th><th>${t('common.colCreatedAt')}</th><th>${t('common.actions')}</th></tr></thead>
        <tbody id="snap-tbody"><tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr></tbody>
      </table>
    </div>
  `);
  await loadSnapshots();
}

let _allSnaps = [];

async function loadSnapshots() {
  const tbody = document.getElementById('snap-tbody');
  try {
    _allSnaps = await api.get('/api/zfs/snapshots');
    renderSnapRows(_allSnaps);
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</td></tr>`;
  }
}

function renderSnapRows(snaps) {
  const tbody = document.getElementById('snap-tbody');
  if (!snaps.length) { tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('zfs.noSnaps')}</td></tr>`; return; }
  tbody.innerHTML = snaps.map(s => `
    <tr>
      <td class="mono">${esc(s.dataset)}</td>
      <td class="mono">${esc(s.snap_name)}</td>
      <td class="mono">${fmtBytes(s.used)}</td>
      <td class="mono">${fmtBytes(s.referenced)}</td>
      <td class="text-dim mono">${fmtTime(s.creation)}</td>
      <td>
        <button class="btn-secondary btn-sm" onclick="window.__fwpCloneSnap('${esc(s.name)}')">${t('zfs.clone')}</button>
        <button class="btn-secondary btn-sm" onclick="window.__fwpRollback('${esc(s.name)}')">${t('zfs.rollback')}</button>
        <button class="btn-danger btn-sm" onclick="window.__fwpDelSnap('${esc(s.name)}')">${t('common.delete')}</button>
      </td>
    </tr>`).join('');
}

window.__fwpSnapFilter = () => {
  const q = (document.getElementById('snap-filter')?.value || '').toLowerCase();
  renderSnapRows(_allSnaps.filter(s => s.dataset.toLowerCase().includes(q) || s.snap_name.toLowerCase().includes(q)));
};


window.__fwpCreateSnap = async () => {
  const result = await formModal(t('zfs.snapCreateTitle'), [
    { key: 'dataset', label: t('zfs.dsLabel'), placeholder: 'zroot/data', required: true },
    { key: 'name', label: t('zfs.snapNameLabel'), placeholder: t('zfs.snapNamePlaceholder'), required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/snapshots', { dataset: result.dataset, name: result.name }).then(() => {
    toast(t('zfs.snapCreatedShort')); loadSnapshots();
  }).catch(e => toast(e.message || t('zfs.snapCreateFailedShort'), 'error'));
};

window.__fwpCloneSnap = async (source) => {
  const result = await formModal(t('zfs.cloneTitle', { name: source }), [
    { key: 'target', label: t('zfs.cloneTargetLabel'), placeholder: t('zfs.cloneTargetPlaceholder'), required: true },
    { key: 'mountpoint', label: t('zfs.cloneMountpointLabel'), placeholder: t('zfs.cloneMountpointPlaceholder') },
  ]);
  if (!result) return;
  api.post('/api/zfs/snapshot/clone', { source, target: result.target, mountpoint: result.mountpoint || undefined }).then(() => {
    toast(t('zfs.cloneDone', { name: result.target })); loadSnapshots();
  }).catch(e => toast(e.message || t('zfs.cloneFailed'), 'error'));
};

window.__fwpDelSnap = async (full) => {
  const result = await confirmDialog(t('zfs.snapDeleteTitle'), t('zfs.snapDeleteConfirm', { name: full }), [
    { key: 'recursive', label: t('zfs.snapRecursive'), checked: false },
  ]);
  if (!result || !result.confirmed) return;
  const qs = `name=${encodeURIComponent(full)}${result.recursive ? '&recursive=true' : ''}`;
  api.del(`/api/zfs/snapshot/destroy?${qs}`).then(() => {
    toast(t('zfs.snapDeleted')); loadSnapshots();
  }).catch(e => toast(e.message || t('zfs.snapDeleteFailed'), 'error'));
};

window.__fwpRollback = async (full) => {
  if (!await confirmDialog(t('zfs.snapRollbackTitle'), t('zfs.snapRollbackConfirm', { name: full }))) return;
  api.post(`/api/zfs/snapshot/rollback?name=${encodeURIComponent(full)}`, { confirm: true }).then(() => {
    toast(t('zfs.snapRollbackDone')); loadSnapshots();
  }).catch(e => toast(e.message || t('zfs.snapRollbackFailed'), 'error'));
};

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function fmtTime(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleString(getLocale());
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
