<script setup>
import { ref, onMounted, onUnmounted, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { Chart, baseOptions, dataIsEmpty, isNotNoiseIface, GRID_COLOR, TICK_COLOR, LABEL_COLOR } from '../lib/chart.js';
import { formatBytesTick, formatRateTick, fmtBytes, fmtRate, fmtTooltipTime } from '../lib/format.js';

const { t } = useI18n();
const ranges = [
  { val: 3600, label: 'monitor.range1h' },
  { val: 21600, label: 'monitor.range6h' },
  { val: 86400, label: 'monitor.range24h' },
  { val: 604800, label: 'monitor.range7d' },
  { val: 2592000, label: 'monitor.range30d' },
];
const rateBuckets = [
  { val: 0, label: 'monitor.rawData' },
  { val: 300, label: '5m' },
  { val: 600, label: '10m' },
  { val: 1800, label: '30m' },
  { val: 3600, label: '1h' },
  { val: 86400, label: '1d' },
];
const buckets = [
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

const msg = ref('');
const ifaces = ref([]);
const charts = {};
const rateState = ref({ range: 86400, bucket: 300, agg: 'avg' });
const trafficState = ref({ range: 86400, bucket: 300 });

const RX_COLOR = '#3b82f6';
const TX_COLOR = '#f59e0b';

async function discoverIfaces() {
  try {
    const latest = await api.get('/api/monitor/latest');
    return (latest.net || [])
      .map((s) => s.name)
      .filter((n) => n.endsWith('.rx'))
      .map((n) => n.slice(0, -3))
      .filter(isNotNoiseIface)
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

/// Fetch every (iface, direction) pair for a section in a single GET.
/// Returns a map name → [[ts, value], ...].  Empty map on any failure —
/// callers just skip drawing and no chart renders.
async function fetchBatch(isTraffic, state) {
  const now = Math.floor(Date.now() / 1000);
  const from = now - state.range;
  const namesParam = ifaces.value.flatMap((iface) => [`${iface}.rx`, `${iface}.tx`]).join(',');
  let url;
  if (isTraffic) {
    if (state.bucket > 0) {
      url = `/api/monitor/aggregate?category=net_bytes&from=${from}&to=${now}&bucket=${state.bucket}&names=${namesParam}`;
    } else {
      url = `/api/monitor/series?category=net_bytes&from=${from}&to=${now}&names=${namesParam}`;
    }
  } else if (state.bucket > 0) {
    url = `/api/monitor/grouped?category=net&from=${from}&to=${now}&bucket=${state.bucket}&agg=${state.agg}&names=${namesParam}`;
  } else {
    url = `/api/monitor/series?category=net&from=${from}&to=${now}&names=${namesParam}`;
  }
  try {
    const res = await api.get(url);
    return res.series || {};
  } catch {
    return {};
  }
}

function drawNetCard(iface, chartId, isTraffic, seriesMap) {
  const rxPoints = seriesMap[`${iface}.rx`] || [];
  const txPoints = seriesMap[`${iface}.tx`] || [];
  const datasets = [
    {
      label: 'RX',
      data: rxPoints.map(([ts, v]) => ({ x: ts * 1000, y: v })),
      backgroundColor: RX_COLOR + 'cc',
      borderColor: RX_COLOR,
      borderWidth: 2,
      pointRadius: 0,
      tension: 0.3,
      fill: false,
    },
    {
      label: 'TX',
      data: txPoints.map(([ts, v]) => ({ x: ts * 1000, y: v })),
      backgroundColor: TX_COLOR + 'cc',
      borderColor: TX_COLOR,
      borderWidth: 2,
      pointRadius: 0,
      tension: 0.3,
      fill: false,
    },
  ];
  if (charts[chartId]) { charts[chartId].destroy(); delete charts[chartId]; }
  const canvas = document.getElementById(chartId);
  if (!canvas || dataIsEmpty(datasets)) return;
  charts[chartId] = new Chart(canvas, { type: 'line', data: { datasets }, options: ifaceOptions(isTraffic) });
}

async function drawAllRate() {
  const seriesMap = await fetchBatch(false, rateState.value);
  for (const iface of ifaces.value) {
    drawNetCard(iface, `chart-rate-${iface}`, false, seriesMap);
  }
}

async function drawAllTraffic() {
  const seriesMap = await fetchBatch(true, trafficState.value);
  for (const iface of ifaces.value) {
    drawNetCard(iface, `chart-traffic-${iface}`, true, seriesMap);
  }
}

function setRateRange(r) {
  rateState.value.range = r;
  drawAllRate();
}

function setRateBucket(b) {
  rateState.value.bucket = b;
  drawAllRate();
}

function setRateAgg(a) {
  rateState.value.agg = a;
  if (rateState.value.bucket > 0) drawAllRate();
}

function setTrafficRange(r) {
  trafficState.value.range = r;
  drawAllTraffic();
}

function setTrafficBucket(b) {
  trafficState.value.bucket = b;
  drawAllTraffic();
}

onMounted(async () => {
  ifaces.value = await discoverIfaces();
  if (!ifaces.value.length) {
    msg.value = t('monitor.noNetData');
    return;
  }
  await nextTick();
  drawAllRate();
  drawAllTraffic();
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
    <div class="toolbar">
      <div class="time-range">
        <button v-for="r in ranges" :key="r.val"
          :class="['btn-secondary', 'btn-sm', { 'active-range': rateState.range === r.val }]"
          @click="setRateRange(r.val)"
        >{{ t(r.label) }}</button>
      </div>
      <div style="display:flex; gap:12px; align-items:center;">
        <div class="filter-group">
          <button v-for="b in rateBuckets" :key="b.val"
            :class="['filter-btn', { active: rateState.bucket === b.val }]"
            @click="setRateBucket(b.val)"
          >{{ b.label.includes('.') ? t(b.label) : b.label }}</button>
        </div>
        <div class="filter-group" v-if="rateState.bucket > 0">
          <button v-for="a in aggMethods" :key="a.val"
            :class="['filter-btn', { active: rateState.agg === a.val }]"
            @click="setRateAgg(a.val)"
          >{{ t(a.label) }}</button>
        </div>
      </div>
    </div>
    <div v-for="iface in ifaces" :key="'rate-'+iface" class="card">
      <div class="card-title">{{ iface }} — {{ t('monitor.viewRate') }}</div>
      <canvas :id="'chart-rate-'+iface" height="120"></canvas>
    </div>

    <h3 class="net-section-title">{{ t('monitor.viewTraffic') }}</h3>
    <div class="toolbar">
      <div class="time-range">
        <button v-for="r in ranges" :key="r.val"
          :class="['btn-secondary', 'btn-sm', { 'active-range': trafficState.range === r.val }]"
          @click="setTrafficRange(r.val)"
        >{{ t(r.label) }}</button>
      </div>
      <div class="filter-group" style="margin-left:auto;">
        <button v-for="b in buckets" :key="b.val"
          :class="['filter-btn', { active: trafficState.bucket === b.val }]"
          @click="setTrafficBucket(b.val)"
        >{{ b.label }}</button>
      </div>
    </div>
    <div v-for="iface in ifaces" :key="'traffic-'+iface" class="card">
      <div class="card-title">{{ iface }} — {{ t('monitor.viewTraffic') }}</div>
      <canvas :id="'chart-traffic-'+iface" height="120"></canvas>
    </div>
  </template>
</template>
