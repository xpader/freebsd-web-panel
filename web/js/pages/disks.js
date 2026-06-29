// Disk management page — detailed per-disk info + partition tables.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { t } from '../i18n/index.js';

export async function renderDisks(app) {
  renderLayout(app, '/filesystem/disks', `
    <div class="page-header">
      <h1>${t('disks.title')}</h1>
      <p>${t('disks.subtitle')}</p>
    </div>
    <div id="disks-content">
      <div class="empty"><span class="spinner"></span> ${t('common.loading')}</div>
    </div>
  `);

  const el = document.getElementById('disks-content');
  let disks;
  try {
    disks = await api.get('/api/filesystem/disks');
  } catch (err) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
    return;
  }

  if (!disks.length) {
    el.innerHTML = `<div class="card empty">${t('fs.noDisks')}</div>`;
    return;
  }

  el.innerHTML = disks.map(diskCard).join('');

  el.querySelectorAll('.uuid-tip').forEach((node) => {
    node.addEventListener('click', async () => {
      const uuid = node.getAttribute('data-uuid');
      try {
        await navigator.clipboard.writeText(uuid);
        toast(t('disks.uuidCopied'));
      } catch {
        toast(t('disks.copyFailed'), 'error');
      }
    });
  });
}

function diskCard(d) {
  const rot = d.rotation_rate === 'unknown' ? t('fs.ssdUnknown') : `${esc(d.rotation_rate)} rpm`;
  const schemeBadge = d.scheme
    ? `<span class="badge badge-dim">${esc(d.scheme)}</span>`
    : `<span class="badge badge-dim">${t('disks.noPartitionTable')}</span>`;
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
    : `<tr><td colspan="7" class="empty">${t('disks.noPartitions')}</td></tr>`;

  const usedBytes = d.partitions.reduce((s, p) => s + p.mediasize_bytes, 0);
  const freeBytes = d.size_bytes > usedBytes ? d.size_bytes - usedBytes : 0;
  const usedPct = d.size_bytes > 0 ? (usedBytes / d.size_bytes) * 100 : 0;

  return `
    <div class="card" style="padding:0;">
      <div class="flex" style="justify-content:space-between;align-items:center;padding:14px 18px;border-bottom:1px solid var(--border);">
        <div class="flex" style="align-items:center;gap:8px;">
          <span class="mono" style="font-size:18px;font-weight:700;">${esc(d.name)}</span>
          <span class="text-dim">·</span>
          <span>${esc(d.descr) || `<span class="text-dim">${t('disks.unknownModel')}</span>`}</span>
        </div>
        <div class="flex" style="align-items:center;gap:8px;">
          ${schemeBadge}
          ${stateBadge}
          <span class="text-dim mono" style="font-size:13px;">${fmtBytes(d.size_bytes)}</span>
        </div>
      </div>

      <div class="stat-grid" style="margin:16px 18px;">
        ${kv(t('disks.devicePath'), `/dev/${d.name}`, 'mono')}
        ${kv(t('disks.model'), d.descr || '—')}
        ${kv(t('disks.totalSize'), fmtBytes(d.size_bytes), 'mono')}
        ${kv(t('disks.sectorSize'), d.sectorsize ? `${d.sectorsize} B` : '—', 'mono')}
        ${kv(t('disks.serialIdent'), d.ident || '—', 'mono')}
        ${kv(t('disks.lunId'), d.lunid || '—', 'mono')}
        ${kv(t('disks.rpm'), rot)}
        ${kv(t('disks.accessMode'), d.mode || '—', 'mono')}
        ${kv(t('disks.fwSectors'), d.fwsectors ? `${d.fwsectors}` : '—', 'mono')}
        ${kv(t('disks.fwHeads'), d.fwheads ? `${d.fwheads}` : '—', 'mono')}
        ${d.scheme ? kv(t('disks.partScheme'), d.scheme) : ''}
        ${d.entries != null ? kv(t('disks.gptEntries'), `${d.entries}`) : ''}
        ${d.first != null ? kv(t('disks.firstSector'), `${d.first}`, 'mono') : ''}
        ${d.last != null ? kv(t('disks.lastSector'), `${d.last}`, 'mono') : ''}
      </div>

      <div style="padding:0 18px 16px;">
        <div class="flex" style="justify-content:space-between;font-size:12px;margin-bottom:6px;">
          <span class="text-dim">${t('disks.allocated', { used: fmtBytes(usedBytes), free: fmtBytes(freeBytes) })}</span>
          <span class="mono text-dim">${usedPct.toFixed(0)}%</span>
        </div>
        <div class="bar-wrap">
          <div class="bar ${usedPct > 80 ? 'bar-swap' : 'bar-cpu'}" style="width:${usedPct}%"></div>
        </div>
      </div>

      <div style="padding:0 18px 18px;">
        <h2 style="font-size:14px;margin:8px 0 8px;">${t('disks.partTable', { n: d.partitions.length })}</h2>
        <table>
          <thead><tr><th>${t('common.device')}</th><th>${t('common.type')}</th><th>${t('disks.colLabel')}</th><th>${t('common.size')}</th><th>${t('disks.colStartSector')}</th><th>${t('disks.colEndSector')}</th><th>${t('disks.colUuid')}</th></tr></thead>
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
