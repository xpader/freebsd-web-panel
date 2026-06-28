// Dashboard — system overview with live metrics.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';

let pollTimer = null;

export async function renderDashboard(app) {
  renderLayout(app, '/dashboard', `
    <div class="page-header">
      <h1>仪表盘</h1>
      <p>系统概览与实时资源状态</p>
    </div>
    <div id="dash-content">
      <div class="empty"><span class="spinner"></span> 加载中…</div>
    </div>
  `);

  const el = document.getElementById('dash-content');
  let info;
  try {
    info = await api.get('/api/system/info');
  } catch (err) {
    el.innerHTML = `<div class="empty">加载失败：${esc(err.message || '')}</div>`;
    return;
  }

  el.innerHTML = `
    <div class="stat-grid">
      <div class="card"><div class="card-title">主机名</div><div class="card-value sm">${esc(info.hostname)}</div></div>
      <div class="card"><div class="card-title">操作系统</div><div class="card-value sm">${esc(info.os_release)}</div></div>
      <div class="card"><div class="card-title">CPU</div><div class="card-value sm">${info.cpu_cores} 核 · ${esc(info.cpu_model)}</div></div>
      <div class="card"><div class="card-title">总内存</div><div class="card-value sm">${fmtBytes(info.memory_total)}</div></div>
      <div class="card"><div class="card-title">运行时间</div><div class="card-value sm" id="m-uptime">—</div></div>
      <div class="card"><div class="card-title">系统负载</div><div class="card-value sm" id="m-loadavg">—</div></div>
    </div>

    <div class="metric-grid">
      <div class="card">
        <div class="card-title">CPU 使用率 <span id="m-cpu-freq" class="text-dim mono" style="font-size:11px;float:right;"></span></div>
        <div class="big-pct" id="m-cpu">—</div>
        <div class="bar-wrap"><div class="bar bar-cpu" id="m-cpu-bar"></div></div>
        <div id="m-cpu-cores" class="core-bars"></div>
      </div>
      <div class="card">
        <div class="card-title">内存使用</div>
        <div class="big-pct" id="m-mem">—</div>
        <div class="bar-wrap"><div class="bar bar-mem" id="m-mem-bar"></div></div>
        <div class="metric-detail" id="m-mem-detail">—</div>
      </div>
      <div class="card">
        <div class="card-title">Swap 使用</div>
        <div class="big-pct" id="m-swap">—</div>
        <div class="bar-wrap"><div class="bar bar-swap" id="m-swap-bar"></div></div>
        <div class="metric-detail" id="m-swap-detail">—</div>
      </div>
      <div class="card">
        <div class="card-title">CPU 温度</div>
        <div id="m-temps"><div class="text-dim">无数据</div></div>
      </div>
    </div>

    <div class="card">
      <div class="card-title">模块状态</div>
      <table>
        <thead><tr><th>模块</th><th>状态</th><th>说明</th></tr></thead>
        <tbody>
          ${MODULES.map(m => `<tr><td><a href="#${m.path}">${m.label}</a></td><td><span class="badge ${m.badge}">${m.status}</span></td><td class="text-dim">${m.note}</td></tr>`).join('')}
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
  setText('m-mem-detail', `${fmtBytes(m.memory.used)} / ${fmtBytes(m.memory.total)} · wired ${fmtBytes(m.memory.wired)}`);

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
      tempsEl.innerHTML = `<div class="text-dim">无传感器数据</div>`;
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
  return d > 0 ? `${d}天 ${h}小时 ${m}分` : `${h}小时 ${m}分`;
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

const MODULES = [
  { path: '/sysctl', label: 'Sysctl 系统参数', status: '计划中', badge: 'badge-warn', note: '动态内核参数管理' },
  { path: '/rcconf', label: 'RC 配置', status: '计划中', badge: 'badge-warn', note: 'rc.conf 服务与系统配置' },
  { path: '/network', label: '网络', status: '计划中', badge: 'badge-warn', note: '网络接口与路由' },
  { path: '/services', label: '服务', status: '计划中', badge: 'badge-warn', note: 'rc.d 服务控制' },
  { path: '/pf', label: '防火墙 (pf)', status: '计划中', badge: 'badge-warn', note: 'PF 规则与状态' },
  { path: '/jails', label: 'Jail 容器', status: '计划中', badge: 'badge-warn', note: 'libjail 原生管理' },
  { path: '/bhyve', label: 'Bhyve 虚拟机', status: '计划中', badge: 'badge-warn', note: 'vm-bhyve 封装' },
  { path: '/zfs', label: 'ZFS 文件系统', status: '计划中', badge: 'badge-warn', note: 'Pool/Dataset/快照' },
];
