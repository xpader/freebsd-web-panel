<script setup>
import { ref, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes, fmtRate, fmtUptime } from '../lib/format.js';

const { t } = useI18n();
const info = ref(null);
const loadError = ref('');
const m = ref(null);
let timer = null;

async function refreshMetrics() {
  try {
    m.value = await api.get('/api/system/metrics');
  } catch { /* ignore */ }
}

function tempClass(temp) {
  return temp >= 70 ? 'badge-danger' : temp >= 55 ? 'badge-warn' : 'badge-success';
}

function memParts() {
  if (!m.value) return [];
  const total = m.value.memory.total || 1;
  return [
    { label: 'Active', val: m.value.memory.active, color: '#8b5cf6' },
    { label: 'Wired', val: m.value.memory.wired, color: '#f59e0b' },
    { label: 'Inact', val: m.value.memory.inactive, color: '#6366f1' },
    { label: 'Laundry', val: m.value.memory.laundry, color: '#06b6d4' },
    { label: 'Cache', val: m.value.memory.cache, color: '#22c55e' },
    { label: 'Free', val: m.value.memory.free_count, color: '#374151' },
  ].map((p) => ({ ...p, pct: (p.val / total * 100).toFixed(1) }));
}

function tempMap() {
  const map = {};
  if (m.value?.temperatures?.length) {
    for (const tmp of m.value.temperatures) {
      const idx = parseInt(tmp.source.replace(/\D/g, ''), 10);
      if (!Number.isNaN(idx)) map[idx] = tmp.value;
    }
  }
  return map;
}

onMounted(async () => {
  try {
    info.value = await api.get('/api/system/info');
  } catch (err) {
    loadError.value = err.message || '';
    return;
  }
  await refreshMetrics();
  timer = setInterval(refreshMetrics, 3000);
});

onUnmounted(() => {
  clearInterval(timer);
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('dash.title') }}</h1>
    <p>{{ t('dash.subtitle') }}</p>
  </div>

  <div v-if="loadError" class="empty">{{ t('common.loadFailed', { msg: loadError }) }}</div>

  <template v-else-if="info && m">
    <!-- System info cards -->
    <div class="stat-grid">
      <div class="card stat-card">
        <div class="card-title"><i class="fa-solid fa-server"></i> {{ t('dash.system') }}</div>
        <div class="stat-row"><span class="stat-label">{{ t('dash.hostname') }}</span><span class="stat-val">{{ info.hostname }}</span></div>
        <div class="stat-row"><span class="stat-label">{{ t('dash.osVersion') }}</span><span class="stat-val">{{ info.os_version }}</span></div>
      </div>
      <div class="card stat-card">
        <div class="card-title"><i class="fa-solid fa-microchip"></i> {{ t('dash.hardware') }}</div>
        <div class="stat-row"><span class="stat-label">CPU</span><span class="stat-val">{{ t('dash.cpuCores', { n: info.cpu_cores, model: info.cpu_model }) }}</span></div>
        <div class="stat-row"><span class="stat-label">{{ t('dash.totalMemory') }}</span><span class="stat-val">{{ fmtBytes(info.memory_total) }}</span></div>
      </div>
      <div class="card stat-card">
        <div class="card-title"><i class="fa-solid fa-gauge-high"></i> {{ t('dash.runtimeStatus') }}</div>
        <div class="stat-row"><span class="stat-label">{{ t('dash.uptime') }}</span><span class="stat-val">{{ fmtUptime(m.uptime_seconds) }}</span></div>
        <div class="stat-row"><span class="stat-label">{{ t('dash.loadavg') }}</span><span class="stat-val">{{ m.loadavg[0].toFixed(2) }} / {{ m.loadavg[1].toFixed(2) }} / {{ m.loadavg[2].toFixed(2) }}</span></div>
      </div>
    </div>

    <!-- Metrics -->
    <div class="metric-grid">
      <!-- CPU -->
      <div class="card">
        <div class="card-title">
          <i class="fa-solid fa-gauge-simple-high"></i> {{ t('dash.cpuUsage') }}
          <span v-if="m.cpu_freq_mhz" class="text-dim mono" style="font-size:11px;float:right;">{{ m.cpu_freq_mhz }} MHz</span>
        </div>
        <div class="big-pct">{{ m.cpu_usage.toFixed(1) }}%</div>
        <div class="bar-wrap"><div class="bar bar-cpu" :style="{ width: Math.min(100, m.cpu_usage) + '%' }"></div></div>
        <div class="core-bars" style="margin-top:12px;">
          <div class="core-bar core-header">
            <span class="core-label">{{ t('common.core') }}</span>
            <span class="core-usage-head">{{ t('common.usage') }}</span>
            <template v-if="m.temperatures && m.temperatures.length"><span class="core-temp">{{ t('common.temp') }}</span></template>
          </div>
          <div v-for="(pct, i) in m.cpu_usage_per_core" :key="i" class="core-bar">
            <span class="core-label">{{ i }}</span>
            <div class="bar-wrap sm"><div class="bar bar-cpu" :style="{ width: pct + '%' }"></div></div>
            <template v-if="m.temperatures && m.temperatures.length">
              <span class="core-temp">
                <span v-if="tempMap()[i] != null" :class="['badge', tempClass(tempMap()[i])]" style="min-width:48px;text-align:center;">{{ tempMap()[i].toFixed(1) }}°C</span>
                <span v-else class="text-dim" style="font-size:11px;">—</span>
              </span>
            </template>
          </div>
        </div>
      </div>

      <!-- Memory -->
      <div class="card">
        <div class="card-title"><i class="fa-solid fa-memory"></i> {{ t('dash.memoryUsage') }}</div>
        <div class="big-pct">{{ m.memory.usage.toFixed(1) }}%</div>
        <div class="bar-wrap"><div class="bar bar-mem" :style="{ width: Math.min(100, m.memory.usage) + '%' }"></div></div>
        <div class="mem-breakdown">
          <div class="mem-stacked">
            <div
              v-for="p in memParts()"
              :key="p.label"
              class="mem-seg"
              :style="{ width: p.pct + '%', background: p.color }"
              :title="`${p.label}: ${fmtBytes(p.val)}`"
            ></div>
          </div>
          <div v-for="p in memParts()" :key="p.label" class="mem-item">
            <span class="mem-dot" :style="{ background: p.color }"></span>
            <span>{{ p.label }}</span>
            <span class="mem-val mono">{{ fmtBytes(p.val) }}</span>
          </div>
        </div>
      </div>

      <!-- Swap -->
      <div class="card">
        <div class="card-title"><i class="fa-solid fa-hard-drive"></i> {{ t('dash.swapUsage') }}</div>
        <div class="big-pct">{{ m.swap.usage.toFixed(1) }}%</div>
        <div class="bar-wrap"><div class="bar bar-swap" :style="{ width: Math.min(100, m.swap.usage) + '%' }"></div></div>
        <div class="metric-detail">{{ fmtBytes(m.swap.used) }} / {{ fmtBytes(m.swap.total) }}</div>
      </div>
    </div>

    <!-- Network -->
    <div class="card">
      <div class="card-title">{{ t('dash.network') }}</div>
      <div v-if="m.network && m.network.length">
        <div v-for="iface in m.network" :key="iface.name" class="dash-net-item">
          <div class="dash-net-head">
            <span class="dash-net-name mono"><i class="fa-solid fa-ethernet"></i> {{ iface.name }}</span>
            <span :class="['badge', iface.up ? 'badge-success' : 'badge-dim']">{{ iface.status || (iface.up ? t('dash.netActive') : t('dash.netDown')) }}</span>
            <span class="dash-net-rates">
              <span class="dash-net-rate dash-net-rx">↓ {{ fmtRate(iface.rx_rate) }}</span>
              <span class="dash-net-rate dash-net-tx">↑ {{ fmtRate(iface.tx_rate) }}</span>
            </span>
          </div>
          <div class="dash-net-bottom">
            <span v-if="(iface.ipv4 || []).length" class="dash-net-ip text-dim mono">IPv4 {{ iface.ipv4.join(', ') }}</span>
            <span v-if="(iface.ipv6 || []).length" class="dash-net-ip text-dim mono">IPv6 {{ iface.ipv6.join(', ') }}</span>
            <span v-if="iface.media" class="text-dim" style="font-size:11px;">{{ iface.media }}</span>
            <span class="dash-net-total text-dim">{{ t('dash.netTotal', { rx: fmtBytes(iface.rx_bytes), tx: fmtBytes(iface.tx_bytes) }) }}</span>
          </div>
        </div>
      </div>
      <div v-else class="text-dim">{{ t('dash.noNet') }}</div>
    </div>

    <footer class="dash-footer">
      <span>FreeBSD Web Panel (fwp)</span>
      <span class="text-dim">© 2026</span>
      <a href="https://github.com/xpader/freebsd-web-panel" target="_blank" rel="noopener"><img src="/img/github.svg" width="16" height="16" class="github-icon" alt="GitHub"> GitHub</a>
    </footer>
  </template>

  <div v-else class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
</template>
