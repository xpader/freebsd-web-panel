// Dashboard — system overview with live metrics.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t, getLocale } from '../i18n/index.js';

let pollTimer = null;

export async function renderDashboard(app) {
  renderLayout(app, '/dashboard', `
    <div class="page-header">
      <h1>${t('dash.title')}</h1>
      <p>${t('dash.subtitle')}</p>
    </div>
    <div id="dash-content">
      <div class="empty"><span class="spinner"></span> ${t('common.loading')}</div>
    </div>
  `);

  const el = document.getElementById('dash-content');
  let info;
  try {
    info = await api.get('/api/system/info');
  } catch (err) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
    return;
  }

  el.innerHTML = `
    <div class="stat-grid">
      <div class="card"><div class="card-title">${t('dash.hostname')}</div><div class="card-value sm">${esc(info.hostname)}</div></div>
      <div class="card"><div class="card-title">${t('dash.os')}</div><div class="card-value sm">${esc(info.os_release)}</div></div>
      <div class="card"><div class="card-title">CPU</div><div class="card-value sm">${t('dash.cpuCores', { n: info.cpu_cores, model: esc(info.cpu_model) })}</div></div>
      <div class="card"><div class="card-title">${t('dash.totalMemory')}</div><div class="card-value sm">${fmtBytes(info.memory_total)}</div></div>
      <div class="card"><div class="card-title">${t('dash.uptime')}</div><div class="card-value sm" id="m-uptime">—</div></div>
      <div class="card"><div class="card-title">${t('dash.loadavg')}</div><div class="card-value sm" id="m-loadavg">—</div></div>
    </div>

    <div class="metric-grid">
      <div class="card">
        <div class="card-title">${t('dash.cpuUsage')} <span id="m-cpu-freq" class="text-dim mono" style="font-size:11px;float:right;"></span></div>
        <div class="big-pct" id="m-cpu">—</div>
        <div class="bar-wrap"><div class="bar bar-cpu" id="m-cpu-bar"></div></div>
        <div id="m-cpu-cores" class="core-bars" style="margin-top:12px;"></div>
      </div>
      <div class="card">
        <div class="card-title">${t('dash.memoryUsage')}</div>
        <div class="big-pct" id="m-mem">—</div>
        <div class="bar-wrap"><div class="bar bar-mem" id="m-mem-bar"></div></div>
        <div class="mem-breakdown" id="m-mem-breakdown"></div>
      </div>
      <div class="card">
        <div class="card-title">${t('dash.swapUsage')}</div>
        <div class="big-pct" id="m-swap">—</div>
        <div class="bar-wrap"><div class="bar bar-swap" id="m-swap-bar"></div></div>
        <div class="metric-detail" id="m-swap-detail">—</div>
      </div>
    </div>

    <div class="card">
      <div class="card-title">${t('dash.network')}</div>
      <div id="m-network"><div class="text-dim">${t('common.loading')}</div></div>
    </div>

    <div class="card">
      <div class="card-title">${t('common.moduleStatus')}</div>
      <table>
        <thead><tr><th>${t('dash.module')}</th><th>${t('common.status')}</th><th>${t('dash.note')}</th></tr></thead>
        <tbody>
          ${MODULES.map(m => `<tr><td><a href="#${m.path}">${t(m.labelKey)}</a></td><td><span class="badge ${m.badge}">${t('status.planned')}</span></td><td class="text-dim">${t(m.noteKey)}</td></tr>`).join('')}
        </tbody>
      </table>
    </div>`;

  // Stop any previous polling, then start fresh.
  clearInterval(pollTimer);
  await refreshMetrics();
  pollTimer = setInterval(refreshMetrics, 3000);
}

async function refreshMetrics() {
  let m;
  try {
    m = await api.get('/api/system/metrics');
  } catch {
    return;
  }

  setText('m-uptime', fmtUptime(m.uptime_seconds));
  setText('m-loadavg', `${m.loadavg[0].toFixed(2)} / ${m.loadavg[1].toFixed(2)} / ${m.loadavg[2].toFixed(2)}`);

  setText('m-cpu', `${m.cpu_usage.toFixed(1)}%`);
  setBar('m-cpu-bar', m.cpu_usage);
  setText('m-cpu-freq', m.cpu_freq_mhz ? `${m.cpu_freq_mhz} MHz` : '');

  const coresEl = document.getElementById('m-cpu-cores');
  if (coresEl) {
    const tempMap = {};
    if (m.temperatures && m.temperatures.length) {
      for (const tmp of m.temperatures) {
        const idx = parseInt(tmp.source.replace(/\D/g, ''), 10);
        if (!Number.isNaN(idx)) tempMap[idx] = tmp.value;
      }
    }
    const hasTemps = m.temperatures && m.temperatures.length > 0;
    const header = `<div class="core-bar core-header"><span class="core-label">${t('common.core')}</span><span class="core-usage-head">${t('common.usage')}</span>${hasTemps ? `<span class="core-temp">${t('common.temp')}</span>` : ''}</div>`;
    coresEl.innerHTML = header + m.cpu_usage_per_core.map((pct, i) => {
      const temp = tempMap[i];
      let tempHtml = '<span class="text-dim" style="font-size:11px;">—</span>';
      if (temp != null) {
        const cls = temp >= 70 ? 'badge-danger' : temp >= 55 ? 'badge-warn' : 'badge-success';
        tempHtml = `<span class="badge ${cls}" style="min-width:48px;text-align:center;">${temp.toFixed(1)}°C</span>`;
      }
      return `<div class="core-bar">
        <span class="core-label">${i}</span>
        <div class="bar-wrap sm"><div class="bar bar-cpu" style="width:${pct}%"></div></div>
        ${hasTemps ? `<span class="core-temp">${tempHtml}</span>` : ''}
      </div>`;
    }).join('');
  }

  setText('m-mem', `${m.memory.usage.toFixed(1)}%`);
  setBar('m-mem-bar', m.memory.usage);
  const memEl = document.getElementById('m-mem-breakdown');
  if (memEl) {
    const total = m.memory.total || 1;
    const parts = [
      { label: 'Active', val: m.memory.active, color: '#8b5cf6' },
      { label: 'Wired', val: m.memory.wired, color: '#f59e0b' },
      { label: 'Inact', val: m.memory.inactive, color: '#6366f1' },
      { label: 'Laundry', val: m.memory.laundry, color: '#06b6d4' },
      { label: 'Cache', val: m.memory.cache, color: '#22c55e' },
      { label: 'Free', val: m.memory.free_count, color: '#374151' },
    ];
    memEl.innerHTML = `<div class="mem-stacked">${parts.map(p => {
      const pct = (p.val / total * 100).toFixed(1);
      return `<div class="mem-seg" style="width:${pct}%;background:${p.color};" title="${p.label}: ${fmtBytes(p.val)}"></div>`;
    }).join('')}</div>` + parts.map(p =>
      `<div class="mem-item"><span class="mem-dot" style="background:${p.color};"></span><span>${p.label}</span><span class="mem-val mono">${fmtBytes(p.val)}</span></div>`
    ).join('');
  }

  setText('m-swap', `${m.swap.usage.toFixed(1)}%`);
  setBar('m-swap-bar', m.swap.usage);
  setText('m-swap-detail', `${fmtBytes(m.swap.used)} / ${fmtBytes(m.swap.total)}`);

  // Network interfaces
  const netEl = document.getElementById('m-network');
  if (netEl) {
    if (m.network && m.network.length) {
      netEl.innerHTML = m.network.map(iface => {
        const statusText = iface.status || (iface.up ? t('dash.netActive') : t('dash.netDown'));
        const statusCls = iface.up ? 'badge-success' : 'badge-dim';
        const ip = (iface.ipv4 || []).join(', ');
        return `<div class="net-iface">
          <div class="net-iface-head">
            <span class="net-name mono">${esc(iface.name)}</span>
            <span class="badge ${statusCls}">${esc(statusText)}</span>
            ${ip ? `<span class="net-ip text-dim mono">${esc(ip)}</span>` : ''}
            ${iface.media ? `<span class="text-dim" style="font-size:11px;">${esc(iface.media)}</span>` : ''}
          </div>
          <div class="net-rates">
            <span class="net-rate net-rx">↓ ${fmtRate(iface.rx_rate)}</span>
            <span class="net-rate net-tx">↑ ${fmtRate(iface.tx_rate)}</span>
            <span class="net-total text-dim">${t('dash.netTotal', { rx: fmtBytes(iface.rx_bytes), tx: fmtBytes(iface.tx_bytes) })}</span>
          </div>
        </div>`;
      }).join('');
    } else {
      netEl.innerHTML = `<div class="text-dim">${t('dash.noNet')}</div>`;
    }
  }
}

function setText(id, val) {
  const el = document.getElementById(id);
  if (el) el.textContent = val;
}
function setBar(id, pct) {
  const el = document.getElementById(id);
  if (el) el.style.width = `${Math.min(100, Math.max(0, pct))}%`;
}

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function fmtRate(bps) {
  if (!bps || bps < 1) return '0 B/s';
  const u = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  let i = 0;
  while (bps >= 1024 && i < u.length - 1) { bps /= 1024; i++; }
  return `${bps.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function fmtUptime(s) {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d > 0 ? t('dash.uptimeFmtDHM', { d, h, m }) : t('dash.uptimeFmtHM', { h, m });
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

const MODULES = [
  { path: '/sysctl', labelKey: 'mod.sysctl.label', noteKey: 'mod.sysctl.note', badge: 'badge-warn' },
  { path: '/rcconf', labelKey: 'mod.rcconf.label', noteKey: 'mod.rcconf.note', badge: 'badge-warn' },
  { path: '/network', labelKey: 'mod.network.label', noteKey: 'mod.network.note', badge: 'badge-warn' },
  { path: '/services', labelKey: 'mod.services.label', noteKey: 'mod.services.note', badge: 'badge-warn' },
  { path: '/pf', labelKey: 'mod.pf.label', noteKey: 'mod.pf.note', badge: 'badge-warn' },
  { path: '/jails', labelKey: 'mod.jails.label', noteKey: 'mod.jails.note', badge: 'badge-warn' },
  { path: '/bhyve', labelKey: 'mod.bhyve.label', noteKey: 'mod.bhyve.note', badge: 'badge-warn' },
  { path: '/zfs', labelKey: 'mod.zfs.label', noteKey: 'mod.zfs.note', badge: 'badge-warn' },
];
