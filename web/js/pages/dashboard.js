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
        <div id="m-cpu-cores" class="core-bars"></div>
      </div>
      <div class="card">
        <div class="card-title">${t('dash.memoryUsage')}</div>
        <div class="big-pct" id="m-mem">—</div>
        <div class="bar-wrap"><div class="bar bar-mem" id="m-mem-bar"></div></div>
        <div class="metric-detail" id="m-mem-detail">—</div>
      </div>
      <div class="card">
        <div class="card-title">${t('dash.swapUsage')}</div>
        <div class="big-pct" id="m-swap">—</div>
        <div class="bar-wrap"><div class="bar bar-swap" id="m-swap-bar"></div></div>
        <div class="metric-detail" id="m-swap-detail">—</div>
      </div>
      <div class="card">
        <div class="card-title">${t('dash.cpuTemp')}</div>
        <div id="m-temps"><div class="text-dim">${t('dash.noData')}</div></div>
      </div>
    </div>

    <div class="card">
      <div class="card-title">${t('common.moduleStatus')}</div>
      <table>
        <thead><tr><th>${t('dash.module')}</th><th>${t('dash.status')}</th><th>${t('dash.note')}</th></tr></thead>
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
    coresEl.innerHTML = m.cpu_usage_per_core.map((pct, i) =>
      `<div class="core-bar"><span class="core-label">${i}</span><div class="bar-wrap sm"><div class="bar bar-cpu" style="width:${pct}%"></div></div></div>`
    ).join('');
  }

  setText('m-mem', `${m.memory.usage.toFixed(1)}%`);
  setBar('m-mem-bar', m.memory.usage);
  setText('m-mem-detail', t('dash.memoryDetail', { used: fmtBytes(m.memory.used), total: fmtBytes(m.memory.total), wired: fmtBytes(m.memory.wired) }));

  setText('m-swap', `${m.swap.usage.toFixed(1)}%`);
  setBar('m-swap-bar', m.swap.usage);
  setText('m-swap-detail', `${fmtBytes(m.swap.used)} / ${fmtBytes(m.swap.total)}`);

  const tempsEl = document.getElementById('m-temps');
  if (tempsEl) {
    if (m.temperatures.length) {
      tempsEl.innerHTML = m.temperatures.map(t => {
        const cls = t.value >= 70 ? 'badge-danger' : t.value >= 55 ? 'badge-warn' : 'badge-success';
        return `<div class="temp-row"><span>${esc(t.source)}</span><span class="badge ${cls}">${t.value.toFixed(1)}°C</span></div>`;
      }).join('');
    } else {
      tempsEl.innerHTML = `<div class="text-dim">${t('dash.noSensorData')}</div>`;
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
