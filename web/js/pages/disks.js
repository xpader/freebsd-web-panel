// Disk management page — detailed per-disk info + partition tables.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';

export async function renderDisks(app) {
  renderLayout(app, '/filesystem/disks', `
    <div class="page-header">
      <h1>磁盘</h1>
      <p>各磁盘的详细参数与分区表</p>
    </div>
    <div id="disks-content">
      <div class="empty"><span class="spinner"></span> 加载中…</div>
    </div>
  `);

  const el = document.getElementById('disks-content');
  let disks;
  try {
    disks = await api.get('/api/filesystem/disks');
  } catch (err) {
    el.innerHTML = `<div class="empty">加载失败：${esc(err.message || '')}</div>`;
    return;
  }

  if (!disks.length) {
    el.innerHTML = '<div class="card empty">无磁盘数据</div>';
    return;
  }

  el.innerHTML = disks.map(diskCard).join('');

  el.querySelectorAll('.uuid-tip').forEach((node) => {
    node.addEventListener('click', async () => {
      const uuid = node.getAttribute('data-uuid');
      try {
        await navigator.clipboard.writeText(uuid);
        toast('已复制 UUID');
      } catch {
        toast('复制失败', 'error');
      }
    });
  });
}

function diskCard(d) {
  const rot = d.rotation_rate === 'unknown' ? 'SSD?' : `${esc(d.rotation_rate)} rpm`;
  const schemeBadge = d.scheme
    ? `<span class="badge badge-dim">${esc(d.scheme)}</span>`
    : '<span class="badge badge-dim">无分区表</span>';
  const stateBadge = d.state
    ? `<span class="badge ${d.state === 'OK' ? 'badge-success' : 'badge-warn'}">${esc(d.state)}</span>`
    : '';

  const partRows = d.partitions.length
    ? d.partitions
        .slice()
        .sort((a, b) => a.index - b.index)
        .map(p => `
          <tr>
            <td class="mono"><strong>${esc(p.name)}</strong></td>
            <td><span class="badge badge-dim">${esc(p.type)}</span></td>
            <td>${esc(p.label) || '<span class="text-dim">—</span>'}</td>
            <td class="mono">${fmtBytes(p.mediasize_bytes)}</td>
            <td class="mono text-dim">${p.start}</td>
            <td class="mono text-dim">${p.end}</td>
            <td class="mono text-dim" style="font-size:11px;"><span class="uuid-tip" data-uuid="${esc(p.rawuuid)}">${esc(p.rawuuid).slice(0, 8)}…</span></td>
          </tr>`).join('')
    : `<tr><td colspan="7" class="empty">无分区</td></tr>`;

  const usedBytes = d.partitions.reduce((s, p) => s + p.mediasize_bytes, 0);
  const freeBytes = d.size_bytes > usedBytes ? d.size_bytes - usedBytes : 0;
  const usedPct = d.size_bytes > 0 ? (usedBytes / d.size_bytes) * 100 : 0;

  return `
    <div class="card" style="padding:0;">
      <div class="flex" style="justify-content:space-between;align-items:center;padding:14px 18px;border-bottom:1px solid var(--border);">
        <div class="flex" style="align-items:center;gap:8px;">
          <span class="mono" style="font-size:18px;font-weight:700;">${esc(d.name)}</span>
          <span class="text-dim">·</span>
          <span>${esc(d.descr) || '<span class="text-dim">未知型号</span>'}</span>
        </div>
        <div class="flex" style="align-items:center;gap:8px;">
          ${schemeBadge}
          ${stateBadge}
          <span class="text-dim mono" style="font-size:13px;">${fmtBytes(d.size_bytes)}</span>
        </div>
      </div>

      <div class="stat-grid" style="margin:16px 18px;">
        ${kv('设备路径', `/dev/${d.name}`, 'mono')}
        ${kv('型号', d.descr || '—')}
        ${kv('总容量', fmtBytes(d.size_bytes), 'mono')}
        ${kv('扇区大小', d.sectorsize ? `${d.sectorsize} B` : '—', 'mono')}
        ${kv('序列号 (ident)', d.ident || '—', 'mono')}
        ${kv('LUN ID', d.lunid || '—', 'mono')}
        ${kv('转速', rot)}
        ${kv('访问模式', d.mode || '—', 'mono')}
        ${kv('固件扇区', d.fwsectors ? `${d.fwsectors}` : '—', 'mono')}
        ${kv('固件磁头', d.fwheads ? `${d.fwheads}` : '—', 'mono')}
        ${d.scheme ? kv('分区方案', d.scheme) : ''}
        ${d.entries != null ? kv('GPT 条目上限', `${d.entries}`) : ''}
        ${d.first != null ? kv('起始扇区', `${d.first}`, 'mono') : ''}
        ${d.last != null ? kv('结束扇区', `${d.last}`, 'mono') : ''}
      </div>

      <div style="padding:0 18px 16px;">
        <div class="flex" style="justify-content:space-between;font-size:12px;margin-bottom:6px;">
          <span class="text-dim">已分配 ${fmtBytes(usedBytes)} · 空闲 ${fmtBytes(freeBytes)}</span>
          <span class="mono text-dim">${usedPct.toFixed(0)}%</span>
        </div>
        <div class="bar-wrap">
          <div class="bar ${usedPct > 80 ? 'bar-swap' : 'bar-cpu'}" style="width:${usedPct}%"></div>
        </div>
      </div>

      <div style="padding:0 18px 18px;">
        <h2 style="font-size:14px;margin:8px 0 8px;">分区表 (${d.partitions.length})</h2>
        <table>
          <thead><tr><th>设备</th><th>类型</th><th>标签</th><th>大小</th><th>起始扇区</th><th>结束扇区</th><th>UUID</th></tr></thead>
          <tbody>${partRows}</tbody>
        </table>
      </div>
    </div>`;
}

function kv(label, value, cls = '') {
  return `
    <div>
      <div class="text-dim" style="font-size:11px;margin-bottom:2px;">${label}</div>
      <div class="${cls}" style="font-size:13px;word-break:break-all;">${esc(value)}</div>
    </div>`;
}

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0;
  let v = b;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
