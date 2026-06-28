// Filesystem overview page — disks, mounts, ZFS pools.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';

export async function renderFsOverview(app) {
  renderLayout(app, '/filesystem', `
    <div class="page-header">
      <h1>文件系统概览</h1>
      <p>磁盘设备、挂载点与 ZFS 存储池状态</p>
    </div>
    <div id="fs-content">
      <div class="empty"><span class="spinner"></span> 加载中…</div>
    </div>
  `);

  const el = document.getElementById('fs-content');
  let data;
  try {
    data = await api.get('/api/filesystem/overview');
  } catch (err) {
    el.innerHTML = `<div class="empty">加载失败：${esc(err.message || '')}</div>`;
    return;
  }

  // ZFS pools summary cards.
  const poolCards = data.zpools.map(p => {
    const pct = p.capacity_pct;
    const healthClass = p.health === 'ONLINE' ? 'badge-success' : 'badge-danger';
    return `
      <div class="card">
        <div class="card-title">存储池: ${esc(p.name)}</div>
        <div class="stat-row">
          <span>状态: <span class="badge ${healthClass}">${esc(p.health)}</span></span>
          <span>容量: <strong>${fmtBytes(p.size)}</strong></span>
          <span>已用: ${fmtBytes(p.allocated)} (${pct.toFixed(0)}%)</span>
          <span>空闲: ${fmtBytes(p.free)}</span>
          <span>碎片率: ${p.fragmentation_pct.toFixed(0)}%</span>
          <span>去重: ${p.dedup.toFixed(2)}x</span>
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
          <td>${d.rotation_rate === 'unknown' ? 'SSD?' : esc(d.rotation_rate) + ' rpm'}</td>
        </tr>`).join('')
    : `<tr><td colspan="4" class="empty">无磁盘数据</td></tr>`;

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
      <h1 style="font-size:18px;">ZFS 存储池</h1>
    </div>
    ${poolCards || '<div class="card empty">无 ZFS 存储池</div>'}

    <div class="page-header" style="margin-bottom:16px;">
      <h1 style="font-size:18px;">物理磁盘 (${data.disks.length})</h1>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>设备</th><th>型号</th><th>容量</th><th>转速</th></tr></thead>
        <tbody>${diskRows}</tbody>
      </table>
    </div>

    <div class="page-header" style="margin-bottom:16px;margin-top:32px;">
      <h1 style="font-size:18px;">挂载点 (${data.mounts.length})</h1>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>设备</th><th>挂载点</th><th>类型</th><th>总容量</th><th>已用</th><th>可用</th><th>使用率</th></tr></thead>
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
