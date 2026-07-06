<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtSpeed, fmtExpire } from '../lib/format.js';

const { t } = useI18n();
const interfaces = ref([]);
const routes = ref([]);
const gateway = ref(null);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const detailIface = ref(null);

const physical = computed(() => interfaces.value.filter((i) => i.is_physical));
const others = computed(() => interfaces.value.filter((i) => !i.is_physical));
const routesV4 = computed(() => routes.value.filter((r) => r.family === 'Internet'));
const routesV6 = computed(() => routes.value.filter((r) => r.family === 'Internet6'));

async function load() {
  if (!interfaces.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    [interfaces.value, routes.value, gateway.value] = await Promise.all([
      api.get('/api/network/interfaces'),
      api.get('/api/network/routes'),
      api.get('/api/network/gateway'),
    ]);
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

function linkLabel(iface) {
  if (iface.link_state === 'up') return t('net.linkUp');
  if (iface.link_state === 'down') return t('net.linkDown');
  return t('common.unknown');
}

function showDetail(iface) {
  detailIface.value = iface;
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('net.title') }}</h1>
    <p>{{ t('net.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <span class="text-dim">{{ interfaces.length }} {{ t('common.device') }}</span>
    <div class="flex">
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div v-if="error" class="card" style="padding:1rem;">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="loading" class="card" style="padding:1rem;"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <!-- Interfaces -->
    <template v-if="physical.length">
      <div class="section-title">{{ t('net.physical') }}</div>
      <div class="card-grid">
        <div v-for="iface in physical" :key="iface.name" class="card net-iface">
          <div class="net-iface-header">
            <i :class="['fa-solid', iface.is_loopback ? 'fa-rotate' : 'fa-ethernet', 'net-iface-icon', iface.is_up ? 'up' : 'down']"></i>
            <span class="net-iface-name mono">{{ iface.name }}</span>
            <span class="net-iface-name-spacer"></span>
            <span class="badge">{{ linkLabel(iface) }}</span>
          </div>
          <div class="net-iface-body">
            <div class="kv"><span class="kv-key">IPv4</span><span class="kv-val">
              <div v-for="ip in iface.ipv4" :key="ip.address" :class="{ 'text-dim': ip.is_alias }">
                {{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}
                <span v-if="ip.is_alias" class="badge">{{ t('net.alias') }}</span>
              </div>
              <span v-if="!iface.ipv4.length" class="text-dim">—</span>
            </span></div>
            <div class="kv"><span class="kv-key">IPv6</span><span class="kv-val">
              <div v-for="ip in iface.ipv6" :key="ip.address" :class="{ 'text-dim': ip.is_alias }">
                {{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}
                <span v-if="ip.is_alias" class="badge">{{ t('net.alias') }}</span>
              </div>
              <span v-if="!iface.ipv6.length" class="text-dim">—</span>
            </span></div>
            <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">{{ iface.mac || '—' }}</span></div>
            <div v-if="iface.is_physical && iface.baudrate" class="kv"><span class="kv-key">{{ t('net.speed') }}</span><span class="kv-val">{{ fmtSpeed(iface.baudrate) }}</span></div>
            <div v-if="iface.groups.length" class="kv"><span class="kv-key">{{ t('net.groups') }}</span><span class="kv-val"><span v-for="g in iface.groups" :key="g" class="badge badge-dim">{{ g }}</span></span></div>
          </div>
          <div class="net-iface-footer">
            <button class="btn-secondary btn-sm" @click="showDetail(iface)">{{ t('net.detail') }}</button>
          </div>
        </div>
      </div>
    </template>

    <template v-if="others.length">
      <div class="section-title" :style="{ marginTop: physical.length ? '32px' : '' }">{{ t('net.virtual') }}</div>
      <div class="card-grid">
        <div v-for="iface in others" :key="iface.name" class="card net-iface">
          <div class="net-iface-header">
            <i :class="['fa-solid', iface.is_loopback ? 'fa-rotate' : 'fa-ethernet', 'net-iface-icon', iface.is_up ? 'up' : 'down']"></i>
            <span class="net-iface-name mono">{{ iface.name }}</span>
            <span class="net-iface-name-spacer"></span>
            <span class="badge">{{ linkLabel(iface) }}</span>
          </div>
          <div class="net-iface-body">
            <div class="kv"><span class="kv-key">IPv4</span><span class="kv-val">
              <div v-for="ip in iface.ipv4" :key="ip.address" :class="{ 'text-dim': ip.is_alias }">
                {{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}
              </div>
              <span v-if="!iface.ipv4.length" class="text-dim">—</span>
            </span></div>
            <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">{{ iface.mac || '—' }}</span></div>
            <div v-if="iface.groups.length" class="kv"><span class="kv-key">{{ t('net.groups') }}</span><span class="kv-val"><span v-for="g in iface.groups" :key="g" class="badge badge-dim">{{ g }}</span></span></div>
          </div>
          <div class="net-iface-footer">
            <button class="btn-secondary btn-sm" @click="showDetail(iface)">{{ t('net.detail') }}</button>
          </div>
        </div>
      </div>
    </template>

    <!-- Gateway -->
    <template v-if="gateway">
      <div class="section-title" style="margin-top:32px;">{{ t('net.defaultGateway') }}</div>
      <div class="card" style="padding:1rem;">
        <div class="kv"><span class="kv-key">{{ t('net.defaultGateway') }}</span><span class="kv-val">
          <strong v-if="gateway.gateway" class="mono">{{ gateway.gateway }}</strong>
          <span v-else class="text-dim">{{ t('net.notConfigured') }}</span>
          {{ gateway.interface ? `(${gateway.interface})` : '' }}
        </span></div>
        <div class="kv"><span class="kv-key">{{ t('net.gatewayConfigured') }}</span><span class="kv-val">
          <span v-if="gateway.configured" class="mono">{{ gateway.configured }}</span>
          <span v-else class="text-dim">{{ t('net.notConfigured') }}</span>
        </span></div>
      </div>
    </template>

    <!-- Routes -->
    <div class="section-title" style="margin-top:32px;">{{ t('net.routes') }}</div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('net.destination') }}</th><th>{{ t('net.gateway') }}</th><th>{{ t('common.status') }}</th><th>{{ t('common.device') }}</th><th>{{ t('net.expire') }}</th>
        </tr></thead>
        <tbody>
          <tr class="cron-section-row"><td colspan="5"><div class="cron-section"><span class="cron-section-title">{{ t('net.routesV4') }}</span><span class="cron-section-sub text-dim">{{ routesV4.length }}</span></div></td></tr>
          <tr v-if="!routesV4.length"><td colspan="5" class="empty">{{ t('common.noData') }}</td></tr>
          <tr v-for="(r, i) in routesV4" :key="'v4-'+i">
            <td class="mono">{{ r.destination }}</td>
            <td class="mono">{{ r.gateway }}</td>
            <td>{{ r.flags }}</td>
            <td class="mono">{{ r.interface }}</td>
            <td>{{ fmtExpire(r.expire) || '—' }}</td>
          </tr>
          <tr class="cron-section-row"><td colspan="5"><div class="cron-section"><span class="cron-section-title">{{ t('net.routesV6') }}</span><span class="cron-section-sub text-dim">{{ routesV6.length }}</span></div></td></tr>
          <tr v-if="!routesV6.length"><td colspan="5" class="empty">{{ t('common.noData') }}</td></tr>
          <tr v-for="(r, i) in routesV6" :key="'v6-'+i">
            <td class="mono">{{ r.destination }}</td>
            <td class="mono">{{ r.gateway }}</td>
            <td>{{ r.flags }}</td>
            <td class="mono">{{ r.interface }}</td>
            <td>{{ fmtExpire(r.expire) || '—' }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>

  <!-- Detail modal -->
  <div v-if="detailIface" class="modal-overlay">
    <div class="modal" style="max-width:760px;">
      <h3>{{ detailIface.name }} — {{ t('net.interfaceInfo') }}</h3>
      <div class="kv-grid">
        <div class="kv"><span class="kv-key">{{ t('common.status') }}</span><span class="kv-val">{{ detailIface.is_up ? t('net.linkUp') : t('net.linkDown') }} ({{ detailIface.link_state }})</span></div>
        <div class="kv"><span class="kv-key">{{ t('net.flags') }}</span><span class="kv-val mono">{{ detailIface.flags.join(', ') }}</span></div>
        <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">{{ detailIface.mac || '—' }}</span></div>
        <div class="kv"><span class="kv-key">MTU</span><span class="kv-val">{{ detailIface.mtu }}</span></div>
        <div class="kv"><span class="kv-key">Metric</span><span class="kv-val">{{ detailIface.metric }}</span></div>
        <div v-if="detailIface.groups.length" class="kv"><span class="kv-key">{{ t('net.groups') }}</span><span class="kv-val"><span v-for="g in detailIface.groups" :key="g" class="badge badge-dim">{{ g }}</span></span></div>
      </div>
      <div v-if="detailIface.ipv4.length" style="margin-top:1rem;">
        <h4>IPv4</h4>
        <table>
          <thead><tr><th>{{ t('common.name') }}</th><th>Netmask</th><th>Broadcast</th><th>{{ t('common.type') }}</th></tr></thead>
          <tbody>
            <tr v-for="(ip, i) in detailIface.ipv4" :key="i">
              <td class="mono">{{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}</td>
              <td class="mono">{{ ip.netmask || '—' }}</td>
              <td class="mono">{{ ip.broadcast || '—' }}</td>
              <td>{{ ip.is_alias ? t('net.alias') : '—' }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-if="detailIface.ipv6.length" style="margin-top:1rem;">
        <h4>IPv6</h4>
        <table>
          <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('common.type') }}</th></tr></thead>
          <tbody>
            <tr v-for="(ip, i) in detailIface.ipv6" :key="i">
              <td class="mono">{{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}</td>
              <td>{{ ip.is_alias ? t('net.alias') : '—' }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="detailIface = null">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>
