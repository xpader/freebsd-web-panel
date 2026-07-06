<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { Chart, baseOptions, dataIsEmpty } from '../lib/chart.js';

const { t } = useI18n();
const ranges = [
  { val: 3600, label: 'monitor.range1h' },
  { val: 21600, label: 'monitor.range6h' },
  { val: 86400, label: 'monitor.range24h' },
  { val: 604800, label: 'monitor.range7d' },
  { val: 2592000, label: 'monitor.range30d' },
];
const selectedRange = ref(86400);
const msg = ref('');
const charts = {};
let usageCanvas, bytesCanvas;

async function drawSeries(canvas, category, nameOrNames, from, to, opts) {
  if (!canvas) return;
  const names = Array.isArray(nameOrNames) ? nameOrNames : [nameOrNames];
  const datasets = [];
  for (let i = 0; i < names.length; i++) {
    let res;
    try {
      res = await api.get(`/api/monitor/series?category=${category}&name=${names[i]}&from=${from}&to=${to}`);
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

  await drawSeries(usageCanvas, 'memory', 'usage', from, now, {
    label: t('monitor.memUsage'), color: '#8b5cf6', yMax: 100, yUnit: '%',
  });
  await drawSeries(bytesCanvas, 'memory', ['active', 'wired', 'inactive', 'laundry', 'cache', 'free'], from, now, {
    multi: true,
    labels: ['Active', 'Wired', 'Inact', 'Laundry', 'Cache', 'Free'],
    colors: ['#8b5cf6', '#f59e0b', '#6366f1', '#06b6d4', '#22c55e', '#374151'],
    byteFormat: true,
  });
}

function setRange(r) {
  selectedRange.value = r;
  drawAll();
}

onMounted(async () => {
  usageCanvas = document.getElementById('chart-mem-usage');
  bytesCanvas = document.getElementById('chart-mem-bytes');
  await drawAll();
});

onUnmounted(() => {
  Object.values(charts).forEach((c) => c.destroy());
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('monitor.memTitle') }}</h1>
    <p>{{ t('monitor.memSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <div class="time-range">
      <button v-for="r in ranges" :key="r.val"
        :class="['btn-secondary', 'btn-sm', { 'active-range': selectedRange === r.val }]"
        @click="setRange(r.val)"
      >{{ t(r.label) }}</button>
    </div>
  </div>
  <div class="card">
    <div class="card-title">{{ t('monitor.memUsagePct') }}</div>
    <canvas id="chart-mem-usage" height="120"></canvas>
  </div>
  <div class="card">
    <div class="card-title">{{ t('monitor.memBytes') }}</div>
    <canvas id="chart-mem-bytes" height="120"></canvas>
  </div>
  <div v-if="msg" class="text-dim" style="text-align:center;padding:20px;">{{ msg }}</div>
</template>
