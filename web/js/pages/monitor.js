// Monitoring pages — CPU, load, memory, temperature charts.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';

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

export async function renderMonitorCpu(app) {
  renderLayout(app, '/monitor', `
    <div class="page-header">
      <h1>CPU 与负载监控</h1>
      <p>CPU 使用率与系统负载历史趋势</p>
    </div>
    <div class="toolbar">
      <div class="time-range" id="time-range">
        <button class="btn-secondary btn-sm" data-range="3600">1小时</button>
        <button class="btn-secondary btn-sm" data-range="21600">6小时</button>
        <button class="btn-secondary btn-sm active-range" data-range="86400">24小时</button>
        <button class="btn-secondary btn-sm" data-range="604800">7天</button>
        <button class="btn-secondary btn-sm" data-range="2592000">30天</button>
      </div>
    </div>
    <div class="card">
      <div class="card-title">CPU 使用率 (%)</div>
      <canvas id="chart-cpu" height="120"></canvas>
    </div>
    <div class="card">
      <div class="card-title">系统负载 (load average)</div>
      <canvas id="chart-load" height="120"></canvas>
    </div>
    <div id="monitor-msg" class="text-dim" style="text-align:center;padding:20px;"></div>
  `);

  await initRangeButtons(['cpu', 'load'], 'monitor');
}

// ---- Memory page (/monitor/memory) ----

export async function renderMonitorMemory(app) {
  renderLayout(app, '/monitor/memory', `
    <div class="page-header">
      <h1>内存监控</h1>
      <p>内存使用率历史趋势</p>
    </div>
    <div class="toolbar">
      <div class="time-range" id="time-range">
        <button class="btn-secondary btn-sm" data-range="3600">1小时</button>
        <button class="btn-secondary btn-sm" data-range="21600">6小时</button>
        <button class="btn-secondary btn-sm active-range" data-range="86400">24小时</button>
        <button class="btn-secondary btn-sm" data-range="604800">7天</button>
        <button class="btn-secondary btn-sm" data-range="2592000">30天</button>
      </div>
    </div>
    <div class="card">
      <div class="card-title">内存使用率 (%)</div>
      <canvas id="chart-mem-usage" height="120"></canvas>
    </div>
    <div class="card">
      <div class="card-title">内存用量 (字节)</div>
      <canvas id="chart-mem-bytes" height="120"></canvas>
    </div>
    <div id="monitor-msg" class="text-dim" style="text-align:center;padding:20px;"></div>
  `);

  await initRangeButtons(['memory'], 'memory');
}

// ---- Temperature page (/monitor/temp) ----

export async function renderMonitorTemp(app) {
  renderLayout(app, '/monitor/temp', `
    <div class="page-header">
      <h1>温度监控</h1>
      <p>CPU 各核心温度历史趋势</p>
    </div>
    <div class="toolbar">
      <div class="time-range" id="time-range">
        <button class="btn-secondary btn-sm" data-range="3600">1小时</button>
        <button class="btn-secondary btn-sm" data-range="21600">6小时</button>
        <button class="btn-secondary btn-sm active-range" data-range="86400">24小时</button>
        <button class="btn-secondary btn-sm" data-range="604800">7天</button>
        <button class="btn-secondary btn-sm" data-range="2592000">30天</button>
      </div>
    </div>
    <div class="card">
      <div class="card-title">CPU 核心温度 (°C)</div>
      <canvas id="chart-temp" height="120"></canvas>
    </div>
    <div id="monitor-msg" class="text-dim" style="text-align:center;padding:20px;"></div>
  `);

  await initRangeButtons(['temp'], 'temp');
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
    showMsg('无法加载图表库 (Chart.js)');
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
      label: 'CPU 总体使用率',
      color: '#3b82f6',
      yMax: 100,
      yUnit: '%',
    });
    await drawSeries(Chart, 'chart-load', 'load', ['1', '5', '15'], from, now, {
      multi: true,
      labels: ['1 分钟', '5 分钟', '15 分钟'],
      colors: ['#3b82f6', '#8b5cf6', '#f59e0b'],
    });
  }
  if (categories.includes('memory')) {
    await drawSeries(Chart, 'chart-mem-usage', 'memory', 'usage', from, now, {
      label: '内存使用率',
      color: '#8b5cf6',
      yMax: 100,
      yUnit: '%',
    });
    await drawSeries(Chart, 'chart-mem-bytes', 'memory', ['used', 'wired'], from, now, {
      multi: true,
      labels: ['已用', 'Wired'],
      colors: ['#8b5cf6', '#f59e0b'],
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
      showMsg('暂无温度数据，等待采集器运行…');
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
      showMsg(`查询失败: ${e.message || ''}`);
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
    showMsg('暂无数据，等待采集器运行…');
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
        ticks: {
          color: '#8b94a5',
          callback: opts.byteFormat ? formatBytesTick : (v) => v + (opts.yUnit || ''),
        },
        grid: { color: '#2a2f3a' },
      },
    },
    plugins: {
      legend: { labels: { color: '#e4e7eb', font: { size: 12 } } },
      tooltip: {
        callbacks: opts.byteFormat
          ? { label: (c) => `${c.dataset.label}: ${fmtBytes(c.parsed.y)}` }
          : { label: (c) => `${c.dataset.label}: ${c.parsed.y.toFixed(1)}${opts.yUnit || ''}` },
      },
    },
    interaction: { mode: 'index', intersect: false },
  };
}

function dataIsEmpty(datasets) {
  return datasets.every((d) => !d.data || d.data.length === 0);
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
