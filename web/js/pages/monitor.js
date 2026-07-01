// Monitoring pages — CPU, load, memory, temperature charts.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

// Load Chart.js + date adapter once (UMD globals). Cached after first import.
let chartJsPromise = null;
function loadChartJs() {
  if (window.Chart) return Promise.resolve(window.Chart);
  if (!chartJsPromise) {
    chartJsPromise = loadScript('/vendor/chart.umd.min.js')
      .then(() => loadScript('/vendor/chartjs-adapter-date-fns.bundle.min.js'))
      .then(() => window.Chart);
  }
  return chartJsPromise;
}

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = src;
    s.onload = () => resolve();
    s.onerror = () => reject(new Error(`load ${src} failed`));
    document.head.appendChild(s);
  });
}

// ---- CPU & Load page (/monitor) ----

function rangeButtons(id) {
  return `
      <div class="time-range" id="${id || 'time-range'}">
        <button class="btn-secondary btn-sm" data-range="3600">${t('monitor.range1h')}</button>
        <button class="btn-secondary btn-sm" data-range="21600">${t('monitor.range6h')}</button>
        <button class="btn-secondary btn-sm active-range" data-range="86400">${t('monitor.range24h')}</button>
        <button class="btn-secondary btn-sm" data-range="604800">${t('monitor.range7d')}</button>
        <button class="btn-secondary btn-sm" data-range="2592000">${t('monitor.range30d')}</button>
      </div>`;
}

export async function renderMonitorCpu(app) {
  renderLayout(app, '/monitor', `
    <div class="page-header">
      <h1>${t('monitor.cpuTitle')}</h1>
      <p>${t('monitor.cpuSubtitle')}</p>
    </div>
    <div class="toolbar">
      ${rangeButtons()}
    </div>
    <div class="card">
      <div class="card-title">${t('monitor.cpuUsagePct')}</div>
      <canvas id="chart-cpu" height="120"></canvas>
    </div>
    <div class="card">
      <div class="card-title">${t('monitor.tempCore')}</div>
      <canvas id="chart-temp" height="120"></canvas>
    </div>
    <div class="card">
      <div class="card-title">${t('monitor.loadAvg')}</div>
      <canvas id="chart-load" height="120"></canvas>
    </div>
    <div id="monitor-msg" class="text-dim" style="text-align:center;padding:20px;"></div>
  `);

  await initRangeButtons(['cpu', 'temp', 'load'], 'monitor');
}

// ---- Memory page (/monitor/memory) ----

export async function renderMonitorMemory(app) {
  renderLayout(app, '/monitor/memory', `
    <div class="page-header">
      <h1>${t('monitor.memTitle')}</h1>
      <p>${t('monitor.memSubtitle')}</p>
    </div>
    <div class="toolbar">
      ${rangeButtons()}
    </div>
    <div class="card">
      <div class="card-title">${t('monitor.memUsagePct')}</div>
      <canvas id="chart-mem-usage" height="120"></canvas>
    </div>
    <div class="card">
      <div class="card-title">${t('monitor.memBytes')}</div>
      <canvas id="chart-mem-bytes" height="120"></canvas>
    </div>
    <div id="monitor-msg" class="text-dim" style="text-align:center;padding:20px;"></div>
  `);

  await initRangeButtons(['memory'], 'memory');
}

// ---- Network page (/monitor/network) ----

export async function renderMonitorNetwork(app) {
  renderLayout(app, '/monitor/network', `
    <div class="page-header">
      <h1>${t('monitor.netRateTitle')}</h1>
      <p>${t('monitor.netRateSubtitle')}</p>
    </div>
    <div id="net-charts"></div>
    <div id="monitor-msg" class="text-dim" style="text-align:center;padding:20px;"></div>
  `);

  await initNetworkPage();
}

// ---- Chart rendering logic ----

const CHARTS = {}; // store active Chart instances by key

async function initRangeButtons(categories, page) {
  const container = document.getElementById('time-range');
  if (!container) return;
  let Chart;
  try {
    Chart = await loadChartJs();
  } catch {
    showMsg(t('monitor.chartLoadFailed'));
    return;
  }

  const handler = (e) => {
    const btn = e.target.closest('button[data-range]');
    if (!btn) return;
    container.querySelectorAll('button').forEach((b) => b.classList.remove('active-range'));
    btn.classList.add('active-range');
    const range = parseInt(btn.dataset.range, 10);
    drawAll(Chart, categories, page, range);
  };
  container.addEventListener('click', handler);

  // Initial draw with default (24h).
  const active = container.querySelector('.active-range') || container.querySelector('button');
  const range = parseInt(active.dataset.range, 10);
  drawAll(Chart, categories, page, range);
}

async function drawAll(Chart, categories, page, rangeSec) {
  const now = Math.floor(Date.now() / 1000);
  const from = now - rangeSec;

  if (categories.includes('cpu')) {
    await drawSeries(Chart, 'chart-cpu', 'cpu', 'total', from, now, {
      label: t('monitor.cpuTotal'),
      color: '#3b82f6',
      yMax: 100,
      yUnit: '%',
    });
    await drawSeries(Chart, 'chart-load', 'load', ['1', '5', '15'], from, now, {
      multi: true,
      labels: [t('monitor.load1'), t('monitor.load5'), t('monitor.load15')],
      colors: ['#3b82f6', '#8b5cf6', '#f59e0b'],
    });
  }
  if (categories.includes('memory')) {
    await drawSeries(Chart, 'chart-mem-usage', 'memory', 'usage', from, now, {
      label: t('monitor.memUsage'),
      color: '#8b5cf6',
      yMax: 100,
      yUnit: '%',
    });
    await drawSeries(Chart, 'chart-mem-bytes', 'memory', ['active', 'wired', 'inactive', 'laundry', 'cache', 'free'], from, now, {
      multi: true,
      labels: ['Active', 'Wired', 'Inact', 'Laundry', 'Cache', 'Free'],
      colors: ['#8b5cf6', '#f59e0b', '#6366f1', '#06b6d4', '#22c55e', '#374151'],
      byteFormat: true,
    });
  }
  if (categories.includes('temp')) {
    // Discover temp series names from latest.
    let names = [];
    try {
      const latest = await api.get('/api/monitor/latest');
      names = latest.temp.map((t) => t.name).sort();
    } catch { /* ignore */ }
    if (names.length === 0) {
      showMsg(t('monitor.noTempData'));
      destroyChart('chart-temp');
      return;
    }
    await drawSeries(Chart, 'chart-temp', 'temp', names, from, now, {
      multi: true,
      labels: names.map((n) => n.replace('cpu', 'CPU ')),
      colors: palette(names.length),
      yUnit: '°C',
    });
  }
  if (categories.includes('net')) {
    let names = [];
    try {
      const latest = await api.get('/api/monitor/latest');
      names = (latest.net || [])
        .map((s) => s.name)
        .filter((n) => n.endsWith('.rx'))
        .map((n) => n.slice(0, -3))
        .filter(isPhysicalIface)
        .sort();
    } catch { /* ignore */ }
    if (names.length === 0) {
      showMsg(t('monitor.noNetData'));
      destroyChart('chart-net-rx');
      destroyChart('chart-net-tx');
      return;
    }
    const rxNames = names.map((n) => `${n}.rx`);
    const txNames = names.map((n) => `${n}.tx`);
    await drawSeries(Chart, 'chart-net-rx', 'net', rxNames, from, now, {
      multi: true,
      labels: names,
      colors: palette(names.length),
      byteRateFormat: true,
    });
    await drawSeries(Chart, 'chart-net-tx', 'net', txNames, from, now, {
      multi: true,
      labels: names,
      colors: palette(names.length),
      byteRateFormat: true,
    });
  }
}

// ---- Network: shared helpers ----

async function discoverIfaces() {
  let ifaces = [];
  try {
    const latest = await api.get('/api/monitor/latest');
    ifaces = (latest.net || [])
      .map((s) => s.name)
      .filter((n) => n.endsWith('.rx'))
      .map((n) => n.slice(0, -3))
      .filter(isPhysicalIface)
      .sort();
  } catch { /* ignore */ }
  return ifaces;
}

function renderIfaceContainers(containerId, ifaces) {
  const el = document.getElementById(containerId);
  if (!el) return;
  for (const iface of ifaces) {
    destroyChart(`chart-rate-${iface}`);
    destroyChart(`chart-traffic-${iface}`);
  }

  const rateCards = ifaces.map(iface => {
    const id = `chart-rate-${iface}`;
    return `<div class="card">
      <div class="card-title">${iface} — ${t('monitor.viewRate')}</div>
      <div class="toolbar">${rangeButtons(`range-${id}`)}</div>
      <canvas id="${id}" height="120"></canvas>
    </div>`;
  }).join('');

  const trafficCards = ifaces.map(iface => {
    const id = `chart-traffic-${iface}`;
    return `<div class="card">
      <div class="card-title">${iface} — ${t('monitor.viewTraffic')}</div>
      <div class="toolbar">
        ${rangeButtons(`range-${id}`)}
        <div class="time-range" id="bucket-${id}" style="margin-left:auto;">
          <button class="btn-secondary btn-sm" data-bucket="300">5m</button>
          <button class="btn-secondary btn-sm" data-bucket="600">10m</button>
          <button class="btn-secondary btn-sm active-bucket" data-bucket="1800">30m</button>
          <button class="btn-secondary btn-sm" data-bucket="3600">1h</button>
          <button class="btn-secondary btn-sm" data-bucket="86400">1d</button>
        </div>
      </div>
      <canvas id="${id}" height="120"></canvas>
    </div>`;
  }).join('');

  el.innerHTML = `<h3 class="net-section-title">${t('monitor.viewRate')}</h3>${rateCards}<h3 class="net-section-title">${t('monitor.viewTraffic')}</h3>${trafficCards}`;
}

const RX_COLOR = '#3b82f6';
const TX_COLOR = '#f59e0b';

// each card keeps its own range/bucket state
const netCardState = {};

async function initNetworkPage() {
  let Chart;
  try { Chart = await loadChartJs(); } catch {
    showMsg(t('monitor.chartLoadFailed'));
    return;
  }

  const ifaces = await discoverIfaces();
  if (ifaces.length === 0) { showMsg(t('monitor.noNetData')); return; }

  renderIfaceContainers('net-charts', ifaces);
  hideMsg();

  for (const iface of ifaces) {
    const rateId = `chart-rate-${iface}`;
    const trafficId = `chart-traffic-${iface}`;
    netCardState[rateId] = { range: 86400 };
    netCardState[trafficId] = { range: 86400, bucket: 1800 };

    // Wire range buttons for rate card
    bindRangeButtons(`range-${rateId}`, () => {
      netCardState[rateId].range = getActiveRange(`range-${rateId}`);
      drawNetCard(Chart, iface, rateId, false);
    });
    // Wire range + bucket buttons for traffic card
    bindRangeButtons(`range-${trafficId}`, () => {
      netCardState[trafficId].range = getActiveRange(`range-${trafficId}`);
      drawNetCard(Chart, iface, trafficId, true);
    });
    bindBucketButtons(trafficId, (bucket) => {
      netCardState[trafficId].bucket = bucket;
      drawNetCard(Chart, iface, trafficId, true);
    });

    drawNetCard(Chart, iface, rateId, false);
    drawNetCard(Chart, iface, trafficId, true);
  }
}

function bindRangeButtons(containerId, cb) {
  const c = document.getElementById(containerId);
  if (!c) return;
  c.addEventListener('click', (e) => {
    const btn = e.target.closest('button[data-range]');
    if (!btn) return;
    c.querySelectorAll('button').forEach((b) => b.classList.remove('active-range'));
    btn.classList.add('active-range');
    cb();
  });
}

function bindBucketButtons(chartId, cb) {
  const container = document.getElementById(`bucket-${chartId}`);
  if (!container) return;
  container.addEventListener('click', (e) => {
    const btn = e.target.closest('button[data-bucket]');
    if (!btn) return;
    container.querySelectorAll('button').forEach((b) => b.classList.remove('active-bucket'));
    btn.classList.add('active-bucket');
    cb(parseInt(btn.dataset.bucket, 10));
  });
}

function getActiveRange(containerId) {
  const c = document.getElementById(containerId);
  if (!c) return 86400;
  const active = c.querySelector('.active-range') || c.querySelector('button');
  return parseInt(active?.dataset.range || '86400', 10);
}

async function drawNetCard(Chart, iface, chartId, isTraffic) {
  const state = netCardState[chartId];
  const now = Math.floor(Date.now() / 1000);
  const from = now - state.range;

  const datasets = [];
  try {
    const rxUrl = isTraffic
      ? `/api/monitor/aggregate?category=net&name=${iface}.rx&from=${from}&to=${now}&bucket=${state.bucket}`
      : `/api/monitor/series?category=net&name=${iface}.rx&from=${from}&to=${now}`;
    const txUrl = isTraffic
      ? `/api/monitor/aggregate?category=net&name=${iface}.tx&from=${from}&to=${now}&bucket=${state.bucket}`
      : `/api/monitor/series?category=net&name=${iface}.tx&from=${from}&to=${now}`;
    const rxRes = await api.get(rxUrl);
    datasets.push(makeDirDataset('RX', rxRes.points, RX_COLOR));
    const txRes = await api.get(txUrl);
    datasets.push(makeDirDataset('TX', txRes.points, TX_COLOR));
  } catch (e) { showMsg(t('monitor.queryFailed', { msg: e.message || '' })); return; }

  destroyChart(chartId);
  const canvas = document.getElementById(chartId);
  if (!canvas || dataIsEmpty(datasets)) return;
  CHARTS[chartId] = new Chart(canvas, {
    type: 'line',
    data: { datasets },
    options: ifaceChartOptions(isTraffic),
  });
}

// ---- Shared chart builders ----

function makeDirDataset(dir, points, color) {
  const data = points.map(([ts, v]) => ({ x: ts * 1000, y: v }));
  return {
    label: dir,
    data,
    backgroundColor: color + 'cc',
    borderColor: color,
    borderWidth: 2,
    pointRadius: data.length > 100 ? 0 : 2,
    tension: 0.3,
    fill: false,
  };
}

function ifaceChartOptions(aggregated) {
  return {
    responsive: true,
    maintainAspectRatio: false,
    scales: {
      x: {
        type: 'time',
        time: { displayFormats: { minute: 'HH:mm', hour: 'MM/dd HH:mm', day: 'MM/dd' } },
        ticks: { color: '#8b94a5', maxRotation: 0, autoSkip: true, maxTicksLimit: 8 },
        grid: { color: '#2a2f3a' },
      },
      y: {
        min: 0,
        ticks: { color: '#8b94a5', callback: aggregated ? formatBytesTick : formatRateTick },
        grid: { color: '#2a2f3a' },
      },
    },
    plugins: {
      legend: { labels: { color: '#e4e7eb', font: { size: 12 } } },
      tooltip: {
        callbacks: {
          title: (items) => fmtTooltipTime(items[0].parsed.x),
          label: (c) => `${c.dataset.label}: ${aggregated ? fmtBytes(c.parsed.y) : fmtRate(c.parsed.y)}`,
        },
      },
    },
    interaction: { mode: 'nearest', axis: 'x', intersect: false },
  };
}

async function drawSeries(Chart, canvasId, category, nameOrNames, from, to, opts) {
  const canvas = document.getElementById(canvasId);
  if (!canvas) return;

  const names = Array.isArray(nameOrNames) ? nameOrNames : [nameOrNames];
  const datasets = [];
  for (let i = 0; i < names.length; i++) {
    let res;
    try {
      res = await api.get(`/api/monitor/series?category=${category}&name=${names[i]}&from=${from}&to=${to}`);
    } catch (e) {
      showMsg(t('monitor.queryFailed', { msg: e.message || '' }));
      return;
    }
    const data = res.points.map(([ts, v]) => ({ x: ts * 1000, y: v }));
    datasets.push({
      label: opts.multi ? opts.labels[i] : opts.label,
      data,
      borderColor: opts.multi ? opts.colors[i] : opts.color,
      backgroundColor: opts.multi ? opts.colors[i] + '20' : opts.color + '20',
      borderWidth: 2,
      pointRadius: data.length > 100 ? 0 : 2,
      tension: 0.3,
      fill: !opts.multi,
    });
  }

  if (dataIsEmpty(datasets)) {
    showMsg(t('monitor.noData'));
    destroyChart(canvasId);
    return;
  }
  hideMsg();

  destroyChart(canvasId);
  CHARTS[canvasId] = new Chart(canvas, {
    type: 'line',
    data: { datasets },
    options: chartOptions(opts),
  });
}

function chartOptions(opts) {
  const tickCb = opts.byteFormat
    ? formatBytesTick
    : opts.byteRateFormat
      ? formatRateTick
      : (v) => v + (opts.yUnit || '');
  const fmtVal = opts.byteFormat
    ? fmtBytes
    : opts.byteRateFormat
      ? fmtRate
      : (v) => v.toFixed(1) + (opts.yUnit || '');
  const tooltipLabel = (c) => `${c.dataset.label}: ${fmtVal(c.parsed.y)}`;
  return {
    responsive: true,
    maintainAspectRatio: false,
    scales: {
      x: {
        type: 'time',
        time: { displayFormats: { minute: 'HH:mm', hour: 'MM/dd HH:mm', day: 'MM/dd' } },
        ticks: { color: '#8b94a5', maxRotation: 0, autoSkip: true, maxTicksLimit: 8 },
        grid: { color: '#2a2f3a' },
      },
      y: {
        min: 0,
        max: opts.yMax || undefined,
        ticks: { color: '#8b94a5', callback: tickCb },
        grid: { color: '#2a2f3a' },
      },
    },
    plugins: {
      legend: { labels: { color: '#e4e7eb', font: { size: 12 } } },
      tooltip: {
        callbacks: {
          title: (items) => fmtTooltipTime(items[0].parsed.x),
          label: tooltipLabel,
        },
      },
    },
    interaction: { mode: 'nearest', axis: 'x', intersect: false },
  };
}

function dataIsEmpty(datasets) {
  return datasets.every((d) => !d.data || d.data.length === 0);
}

function fmtTooltipTime(x) {
  const d = new Date(x);
  const MM = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  const HH = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${MM}/${dd} ${HH}:${mm}`;
}

function destroyChart(id) {
  if (CHARTS[id]) {
    CHARTS[id].destroy();
    delete CHARTS[id];
  }
}

function palette(n) {
  const base = ['#3b82f6', '#8b5cf6', '#22c55e', '#f59e0b', '#ef4444', '#06b6d4', '#ec4899', '#a78bfa'];
  return Array.from({ length: n }, (_, i) => base[i % base.length]);
}

// Same denylist as the backend `is_physical_iface` (src/sysinfo.rs) — kept in
// sync so historical data for virtual interfaces (epair, bridge, tap, VPN…)
// leftover in the DB is not rendered even though the collector no longer
// samples them.
const VIRTUAL_IFACE_PREFIXES = [
  'lo', 'epair', 'bridge', 'tap', 'vale', 'tun', 'gif', 'gre', 'ipfw',
  'pflog', 'pfsync', 'enc', 'stf', 'faith', 'ng', 'vm-', 'tailscale', 'wg', 'disc', 'edsc',
];
function isPhysicalIface(name) {
  return !VIRTUAL_IFACE_PREFIXES.some((p) => name.startsWith(p));
}

function showMsg(msg) {
  const el = document.getElementById('monitor-msg');
  if (el) el.textContent = msg;
}
function hideMsg() {
  const el = document.getElementById('monitor-msg');
  if (el) el.textContent = '';
}

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function formatBytesTick(v) {
  if (v >= 1e9) return (v / 1e9).toFixed(0) + 'GB';
  if (v >= 1e6) return (v / 1e6).toFixed(0) + 'MB';
  if (v >= 1e3) return (v / 1e3).toFixed(0) + 'KB';
  return v + 'B';
}
function fmtRate(bps) {
  if (!bps || bps < 1) return '0 B/s';
  const u = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  let i = 0;
  while (bps >= 1024 && i < u.length - 1) { bps /= 1024; i++; }
  return `${bps.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function formatRateTick(v) {
  if (v >= 1e9) return (v / 1e9).toFixed(1) + 'GB/s';
  if (v >= 1e6) return (v / 1e6).toFixed(0) + 'MB/s';
  if (v >= 1e3) return (v / 1e3).toFixed(0) + 'KB/s';
  return v + 'B/s';
}
