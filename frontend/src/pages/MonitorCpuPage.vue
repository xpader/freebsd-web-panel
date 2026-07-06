<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { Chart, palette, baseOptions, dataIsEmpty, isPhysicalIface, GRID_COLOR, TICK_COLOR, LABEL_COLOR } from '../lib/chart.js';
import { fmtTooltipTime } from '../lib/format.js';

const { t } = useI18n();
const ranges = [
  { val: 3600, label: 'monitor.range1h' },
  { val: 21600, label: 'monitor.range6h' },
  { val: 86400, label: 'monitor.range24h' },
  { val: 604800, label: 'monitor.range7d' },
  { val: 2592000, label: 'monitor.range30d' },
];
const buckets = [
  { val: 0, label: 'monitor.rawData' },
  { val: 300, label: '5m' },
  { val: 600, label: '10m' },
  { val: 1800, label: '30m' },
  { val: 3600, label: '1h' },
  { val: 86400, label: '1d' },
];
const aggMethods = [
  { val: 'min', label: 'monitor.aggMin' },
  { val: 'avg', label: 'monitor.aggAvg' },
  { val: 'max', label: 'monitor.aggMax' },
];
const selectedRange = ref(86400);
const selectedBucket = ref(300);
const selectedAgg = ref('avg');
const msg = ref('');
const charts = {};
let cpuCanvas, tempCanvas, loadCanvas;

async function drawSeries(canvas, category, nameOrNames, from, to, opts) {
  if (!canvas) return;
  const names = Array.isArray(nameOrNames) ? nameOrNames : [nameOrNames];
  const datasets = [];
  for (let i = 0; i < names.length; i++) {
    let res;
    try {
      const url = selectedBucket.value > 0
        ? `/api/monitor/grouped?category=${category}&name=${names[i]}&from=${from}&to=${to}&bucket=${selectedBucket.value}&agg=${selectedAgg.value}`
        : `/api/monitor/series?category=${category}&name=${names[i]}&from=${from}&to=${to}`;
      res = await api.get(url);
    } catch (e) {
      msg.value = t('monitor.queryFailed', { msg: e.message || '' });
      return;
    }
    datasets.push({
      label: opts.multi ? opts.labels[i] : opts.label,
      data: res.points.map(([ts, v]) => ({ x: ts * 1000, y: v })),
      borderColor: opts.multi ? opts.colors[i] : opts.color,
      backgroundColor: opts.multi ? opts.colors[i] + '20' : opts.color + '20',
      borderWidth: 2,
      pointRadius: 0,
      tension: 0.3,
      fill: !opts.multi,
    });
  }
  if (dataIsEmpty(datasets)) {
    msg.value = t('monitor.noData');
    return;
  }
  msg.value = '';
  if (charts[canvas.id]) charts[canvas.id].destroy();
  charts[canvas.id] = new Chart(canvas, { type: 'line', data: { datasets }, options: baseOptions(opts) });
}

async function drawAll() {
  const now = Math.floor(Date.now() / 1000);
  const from = now - selectedRange.value;
  msg.value = '';

  await drawSeries(cpuCanvas, 'cpu', 'total', from, now, {
    label: t('monitor.cpuTotal'), color: '#3b82f6', yMax: 100, yUnit: '%',
  });
  await drawSeries(loadCanvas, 'load', ['1', '5', '15'], from, now, {
    multi: true, labels: [t('monitor.load1'), t('monitor.load5'), t('monitor.load15')],
    colors: ['#3b82f6', '#8b5cf6', '#f59e0b'],
  });

  // Temperature: discover names from latest.
  let names = [];
  try {
    const latest = await api.get('/api/monitor/latest');
    names = latest.temp.map((tp) => tp.name).sort();
  } catch {}
  if (!names.length) {
    if (charts['chart-temp']) { charts['chart-temp'].destroy(); delete charts['chart-temp']; }
  } else {
    await drawSeries(tempCanvas, 'temp', names, from, now, {
      multi: true,
      labels: names.map((n) => n.replace('cpu', 'CPU ')),
      colors: palette(names.length),
      yUnit: '°C',
    });
  }
}

function setRange(r) {
  selectedRange.value = r;
  drawAll();
}

function setBucket(b) {
  selectedBucket.value = b;
  drawAll();
}

function setAgg(a) {
  selectedAgg.value = a;
  if (selectedBucket.value > 0) drawAll();
}

onMounted(async () => {
  cpuCanvas = document.getElementById('chart-cpu');
  tempCanvas = document.getElementById('chart-temp');
  loadCanvas = document.getElementById('chart-load');
  await drawAll();
});

onUnmounted(() => {
  Object.values(charts).forEach((c) => c.destroy());
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('monitor.cpuTitle') }}</h1>
    <p>{{ t('monitor.cpuSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <div class="time-range">
      <button v-for="r in ranges" :key="r.val"
        :class="['btn-secondary', 'btn-sm', { 'active-range': selectedRange === r.val }]"
        @click="setRange(r.val)"
      >{{ t(r.label) }}</button>
    </div>
    <div style="display:flex; gap:12px; align-items:center;">
      <div class="filter-group">
        <button v-for="b in buckets" :key="b.val"
          :class="['filter-btn', { active: selectedBucket === b.val }]"
          @click="setBucket(b.val)"
        >{{ b.label.includes('.') ? t(b.label) : b.label }}</button>
      </div>
      <div class="filter-group" v-if="selectedBucket > 0">
        <button v-for="a in aggMethods" :key="a.val"
          :class="['filter-btn', { active: selectedAgg === a.val }]"
          @click="setAgg(a.val)"
        >{{ t(a.label) }}</button>
      </div>
    </div>
  </div>
  <div class="card">
    <div class="card-title">{{ t('monitor.cpuUsagePct') }}</div>
    <canvas id="chart-cpu" height="120"></canvas>
  </div>
  <div class="card">
    <div class="card-title">{{ t('monitor.tempCore') }}</div>
    <canvas id="chart-temp" height="120"></canvas>
  </div>
  <div class="card">
    <div class="card-title">{{ t('monitor.loadAvg') }}</div>
    <canvas id="chart-load" height="120"></canvas>
  </div>
  <div v-if="msg" class="text-dim" style="text-align:center;padding:20px;">{{ msg }}</div>
</template>
