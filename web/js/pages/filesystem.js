// Filesystem overview page — disks, mounts, ZFS pools.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

export async function renderFsOverview(app) {
  renderLayout(app, '/filesystem', `
    <div class="page-header">
      <h1>${t('fs.title')}</h1>
      <p>${t('fs.subtitle')}</p>
    </div>
    <div id="fs-content">
      <div class="empty"><span class="spinner"></span> ${t('common.loading')}</div>
    </div>
  `);

  const el = document.getElementById('fs-content');
  let data;
  try {
    data = await api.get('/api/filesystem/overview');
  } catch (err) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
    return;
  }

  // ZFS pools summary cards.
  const poolCards = data.zpools.map(p => {
    const pct = p.capacity_pct;
    const healthClass = p.health === 'ONLINE' ? 'badge-success' : 'badge-danger';
    return `
      <div class="card">
        <div class="card-title">${t('fs.poolName', { name: esc(p.name) })}</div>
        <div class="stat-row">
          <span>${t('fs.state')}: <span class="badge ${healthClass}">${esc(p.health)}</span></span>
          <span>${t('common.capacity')}: <strong>${fmtBytes(p.size)}</strong></span>
          <span>${t('common.used')}: ${fmtBytes(p.allocated)} (${pct.toFixed(0)}%)</span>
          <span>${t('common.free')}: ${fmtBytes(p.free)}</span>
          <span>${t('common.frag')}: ${p.fragmentation_pct.toFixed(0)}%</span>
          <span>${t('common.dedup')}: ${p.dedup.toFixed(2)}x</span>
        </div>
        <div class="bar-wrap" style="margin-top:10px;">
          <div class="bar bar-${pct > 80 ? 'swap' : 'mem'}" style="width:${pct}%"></div>
        </div>
      </div>`;
  }).join('');

  // Disk table.
  const diskRows = data.disks.length
    ? data.disks.map(d => `
        <tr>
          <td class="mono"><strong>${esc(d.name)}</strong></td>
          <td>${esc(d.descr)}</td>
          <td class="mono">${fmtBytes(d.size_bytes)}</td>
          <td>${d.rotation_rate === 'unknown' ? t('fs.ssdUnknown') : esc(d.rotation_rate) + ' rpm'}</td>
        </tr>`).join('')
    : `<tr><td colspan="4" class="empty">${t('fs.noDisks')}</td></tr>`;

  // Mount table.
  const mountRows = data.mounts.map(m => `
    <tr>
      <td class="mono">${esc(m.device)}</td>
      <td class="mono">${esc(m.mountpoint)}</td>
      <td><span class="badge badge-dim">${esc(m.fstype)}</span></td>
      <td class="mono">${m.size > 0 ? fmtBytes(m.size) : '—'}</td>
      <td class="mono">${m.size > 0 ? fmtBytes(m.used) : '—'}</td>
      <td class="mono">${m.size > 0 ? fmtBytes(m.available) : '—'}</td>
      <td>${m.size > 0 ? barCell(m.capacity_pct) : '—'}</td>
    </tr>`).join('');

  el.innerHTML = `
    <div class="page-header" style="margin-bottom:16px;">
      <h1 style="font-size:18px;">${t('fs.zfsPools')}</h1>
    </div>
    ${poolCards || `<div class="card empty">${t('fs.noPools')}</div>`}

    <div class="page-header" style="margin-bottom:16px;">
      <h1 style="font-size:18px;">${t('fs.physicalDisks', { n: data.disks.length })}</h1>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('common.device')}</th><th>${t('fs.colModel')}</th><th>${t('fs.colSize')}</th><th>${t('fs.colRpm')}</th></tr></thead>
        <tbody>${diskRows}</tbody>
      </table>
    </div>

    <div class="page-header" style="margin-bottom:16px;margin-top:32px;">
      <h1 style="font-size:18px;">${t('fs.mountpoints', { n: data.mounts.length })}</h1>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>${t('common.device')}</th><th>${t('fs.colMountpoint')}</th><th>${t('fs.colFstype')}</th><th>${t('fs.colTotal')}</th><th>${t('common.used')}</th><th>${t('fs.colAvailable')}</th><th>${t('fs.colUsage')}</th></tr></thead>
        <tbody>${mountRows}</tbody>
      </table>
    </div>`;
}

function barCell(pct) {
  const cls = pct > 80 ? 'bar-swap' : pct > 50 ? 'bar-mem' : 'bar-cpu';
  return `<div class="flex"><div class="bar-wrap sm" style="width:80px;"><div class="bar ${cls}" style="width:${pct}%"></div></div><span class="text-dim mono" style="font-size:11px;">${pct.toFixed(0)}%</span></div>`;
}

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
