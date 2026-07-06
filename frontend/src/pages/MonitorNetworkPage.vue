<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { Chart, baseOptions, dataIsEmpty, isPhysicalIface, GRID_COLOR, TICK_COLOR, LABEL_COLOR } from '../lib/chart.js';
import { formatBytesTick, formatRateTick, fmtBytes, fmtRate, fmtTooltipTime } from '../lib/format.js';

const { t } = useI18n();
const ranges = [
  { val: 3600, label: 'monitor.range1h' },
  { val: 21600, label: 'monitor.range6h' },
  { val: 86400, label: 'monitor.range24h' },
  { val: 604800, label: 'monitor.range7d' },
  { val: 2592000, label: 'monitor.range30d' },
];
const buckets = [
  { val: 300, label: '5m' },
  { val: 600, label: '10m' },
  { val: 1800, label: '30m' },
  { val: 3600, label: '1h' },
  { val: 86400, label: '1d' },
];

const msg = ref('');
const ifaces = ref([]);
const charts = {};
// Per-card state: { [chartId]: { range, bucket } }
const cardState = ref({});

const RX_COLOR = '#3b82f6';
const TX_COLOR = '#f59e0b';

async function discoverIfaces() {
  try {
    const latest = await api.get('/api/monitor/latest');
    return (latest.net || [])
      .map((s) => s.name)
      .filter((n) => n.endsWith('.rx'))
      .map((n) => n.slice(0, -3))
      .filter(isPhysicalIface)
      .sort();
  } catch {
    return [];
  }
}

function ifaceOptions(aggregated) {
  const tickCb = aggregated ? formatBytesTick : formatRateTick;
  const fmtVal = aggregated ? fmtBytes : fmtRate;
  return {
    responsive: true,
    maintainAspectRatio: false,
    scales: {
      x: {
        type: 'time',
        time: { displayFormats: { minute: 'HH:mm', hour: 'MM/dd HH:mm', day: 'MM/dd' } },
        ticks: { color: TICK_COLOR, maxRotation: 0, autoSkip: true, maxTicksLimit: 8 },
        grid: { color: GRID_COLOR },
      },
      y: { min: 0, ticks: { color: TICK_COLOR, callback: tickCb }, grid: { color: GRID_COLOR } },
    },
    plugins: {
      legend: { labels: { color: LABEL_COLOR, font: { size: 12 } } },
      tooltip: {
        callbacks: {
          title: (items) => fmtTooltipTime(items[0].parsed.x),
          label: (c) => `${c.dataset.label}: ${fmtVal(c.parsed.y)}`,
        },
      },
    },
    interaction: { mode: 'nearest', axis: 'x', intersect: false },
  };
}

async function drawNetCard(iface, chartId, isTraffic) {
  const state = cardState.value[chartId];
  if (!state) return;
  const now = Math.floor(Date.now() / 1000);
  const from = now - state.range;
  const datasets = [];
  try {
    for (const [dir, color] of [['rx', RX_COLOR], ['tx', TX_COLOR]]) {
      const url = isTraffic
        ? `/api/monitor/aggregate?category=net&name=${iface}.${dir}&from=${from}&to=${now}&bucket=${state.bucket}`
        : `/api/monitor/series?category=net&name=${iface}.${dir}&from=${from}&to=${now}`;
      const res = await api.get(url);
      datasets.push({
        label: dir.toUpperCase(),
        data: res.points.map(([ts, v]) => ({ x: ts * 1000, y: v })),
        backgroundColor: color + 'cc',
        borderColor: color,
        borderWidth: 2,
        pointRadius: 0,
        tension: 0.3,
        fill: false,
      });
    }
  } catch (e) {
    msg.value = t('monitor.queryFailed', { msg: e.message || '' });
    return;
  }

  if (charts[chartId]) { charts[chartId].destroy(); delete charts[chartId]; }
  const canvas = document.getElementById(chartId);
  if (!canvas || dataIsEmpty(datasets)) return;
  charts[chartId] = new Chart(canvas, { type: 'line', data: { datasets }, options: ifaceOptions(isTraffic) });
}

function setRange(chartId, r) {
  cardState.value[chartId].range = r;
  const iface = chartId.replace('chart-rate-', '').replace('chart-traffic-', '');
  drawNetCard(iface, chartId, chartId.includes('traffic'));
}

function setBucket(chartId, b) {
  cardState.value[chartId].bucket = b;
  const iface = chartId.replace('chart-traffic-', '');
  drawNetCard(iface, chartId, true);
}

onMounted(async () => {
  ifaces.value = await discoverIfaces();
  if (!ifaces.value.length) {
    msg.value = t('monitor.noNetData');
    return;
  }
  for (const iface of ifaces.value) {
    const rateId = `chart-rate-${iface}`;
    const trafficId = `chart-traffic-${iface}`;
    cardState.value[rateId] = { range: 86400 };
    cardState.value[trafficId] = { range: 86400, bucket: 1800 };
  }
  // Wait for DOM to render canvas elements, then draw.
  await new Promise((r) => requestAnimationFrame(r));
  for (const iface of ifaces.value) {
    drawNetCard(iface, `chart-rate-${iface}`, false);
    drawNetCard(iface, `chart-traffic-${iface}`, true);
  }
});

onUnmounted(() => {
  Object.values(charts).forEach((c) => c.destroy());
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('monitor.netRateTitle') }}</h1>
    <p>{{ t('monitor.netRateSubtitle') }}</p>
  </div>

  <div v-if="msg" class="text-dim" style="text-align:center;padding:20px;">{{ msg }}</div>

  <template v-else>
    <h3 class="net-section-title">{{ t('monitor.viewRate') }}</h3>
    <div v-for="iface in ifaces" :key="'rate-'+iface" class="card">
      <div class="card-title">{{ iface }} — {{ t('monitor.viewRate') }}</div>
      <div class="toolbar">
        <div class="time-range">
          <button v-for="r in ranges" :key="r.val"
            :class="['btn-secondary', 'btn-sm', { 'active-range': cardState['chart-rate-'+iface]?.range === r.val }]"
            @click="setRange('chart-rate-'+iface, r.val)"
          >{{ t(r.label) }}</button>
        </div>
      </div>
      <canvas :id="'chart-rate-'+iface" height="120"></canvas>
    </div>

    <h3 class="net-section-title">{{ t('monitor.viewTraffic') }}</h3>
    <div v-for="iface in ifaces" :key="'traffic-'+iface" class="card">
      <div class="card-title">{{ iface }} — {{ t('monitor.viewTraffic') }}</div>
      <div class="toolbar">
        <div class="time-range">
          <button v-for="r in ranges" :key="r.val"
            :class="['btn-secondary', 'btn-sm', { 'active-range': cardState['chart-traffic-'+iface]?.range === r.val }]"
            @click="setRange('chart-traffic-'+iface, r.val)"
          >{{ t(r.label) }}</button>
        </div>
        <div class="time-range" style="margin-left:auto;">
          <button v-for="b in buckets" :key="b.val"
            :class="['btn-secondary', 'btn-sm', { 'active-bucket': cardState['chart-traffic-'+iface]?.bucket === b.val }]"
            @click="setBucket('chart-traffic-'+iface, b.val)"
          >{{ b.label }}</button>
        </div>
      </div>
      <canvas :id="'chart-traffic-'+iface" height="120"></canvas>
    </div>
  </template>
</template>
