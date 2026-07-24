<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtSpeed, fmtExpire } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import FieldHelp from '../components/ui/FieldHelp.vue';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();
const interfaces = ref([]);
const routes = ref([]);
const gateway = ref(null);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const detailIface = ref(null);
const configIfaceName = ref(null);
const configData = ref(null);
const configDriverName = ref(null);
const configLoading = ref(false);
const configSaving = ref(false);
const configApplying = ref(false);
const createDialog = ref(false);
const createType = ref('bridge');
const createNum = ref(0);
const createCustomName = ref('');
const creating = ref(false);

const ifaceTypes = computed(() => [
  { value: 'bridge', label: t('net.ifaceTypeBridge'), prefix: 'bridge' },
  { value: 'lagg', label: t('net.ifaceTypeLagg'), prefix: 'lagg' },
  { value: 'vlan', label: t('net.ifaceTypeVlan'), prefix: 'vlan' },
  { value: 'tap', label: t('net.ifaceTypeTap'), prefix: 'tap' },
  { value: 'epair', label: t('net.ifaceTypeEpair'), prefix: 'epair' },
  { value: 'custom', label: t('net.ifaceTypeCustom'), prefix: '' },
]);

const createPreview = computed(() => {
  const sel = ifaceTypes.value.find((tp) => tp.value === createType.value);
  if (!sel) return '';
  if (sel.value === 'custom') return createCustomName.value.trim();
  if (sel.value === 'epair') {
    const n = createNum.value;
    return `epair${n}a  epair${n}b`;
  }
  return `${sel.prefix}${createNum.value}`;
});

const physical = computed(() => interfaces.value.filter((i) => i.is_physical));
const bridges = computed(() => interfaces.value.filter((i) => !i.is_physical && (i.name.startsWith('bridge') || i.driver_name?.startsWith('bridge'))));
const others = computed(() => interfaces.value.filter((i) => !i.is_physical && !i.name.startsWith('bridge') && !i.driver_name?.startsWith('bridge')));
const routesV4 = computed(() => routes.value.filter((r) => r.family === 'Internet'));
const routesV6 = computed(() => routes.value.filter((r) => r.family === 'Internet6'));

const ipv4Mode = computed({
  get: () => {
    if (!configData.value) return 'none';
    const v = configData.value.ipv4;
    if (v === null || v === undefined) return 'none';
    if (v === 'DHCP' || v === 'SYNCDHCP') return 'dhcp';
    return 'static';
  },
  set: (val) => {
    if (!configData.value) return;
    if (val === 'none') {
      configData.value.ipv4 = null;
    } else if (val === 'dhcp') {
      configData.value.ipv4 = 'DHCP';
    } else {
      const cur = configData.value.ipv4;
      if (!cur || cur === 'DHCP' || cur === 'SYNCDHCP') {
        configData.value.ipv4 = '';
      }
    }
  },
});

const ipv6Mode = computed({
  get: () => {
    if (!configData.value) return 'none';
    const m = configData.value.ipv6_mode;
    if (m === 'slaac') return 'slaac';
    if (m === 'static') return 'static';
    return 'none';
  },
  set: (val) => {
    if (!configData.value) return;
    configData.value.ipv6_mode = val;
    if (val === 'none') configData.value.ipv6 = [];
  },
});

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

function canConfigure(iface) {
  if (iface.status && iface.status.trim()) return false;
  const n = iface.name;
  return n.length >= 1 && n.length <= 15 && /^[a-zA-Z0-9_.]+$/.test(n);
}

function canDestroy(iface) {
  return canConfigure(iface) && !iface.is_physical && !iface.is_loopback;
}

async function showConfig(iface) {
  configIfaceName.value = iface.name;
  configDriverName.value = iface.driver_name || null;
  configData.value = null;
  configLoading.value = true;
  try {
    const resp = await api.get(`/api/network/interfaces/${iface.name}`);
    configData.value = resp;
  } catch (err) {
    await alert(t('common.operationFailed'), err.message || t('common.loadFailed', { msg: '' }));
    configIfaceName.value = null;
  } finally {
    configLoading.value = false;
  }
}

function addAlias() {
  configData.value.ipv4_aliases.push({ address: '', netmask: '' });
}

function removeAlias(i) {
  configData.value.ipv4_aliases.splice(i, 1);
}

function addIpv6() {
  configData.value.ipv6.push({ address: '', prefixlen: '64' });
}

function removeIpv6(i) {
  configData.value.ipv6.splice(i, 1);
}

function addMember() {
  configData.value.bridge_members.push('');
}

const availableBridgeMembers = computed(() => {
  const bridgeName = configIfaceName.value;
  const inOtherBridge = new Set();
  for (const iface of interfaces.value) {
    if (iface.name === bridgeName) continue;
    for (const m of iface.members) {
      inOtherBridge.add(m.name);
    }
  }
  const currentMembers = new Set();
  const currentIface = interfaces.value.find((i) => i.name === bridgeName);
  if (currentIface) {
    for (const m of currentIface.members) {
      currentMembers.add(m.name);
    }
  }
  return interfaces.value.filter((iface) =>
    iface.name !== bridgeName
    && !iface.is_loopback
    && !iface.name.startsWith('bridge')
    && !iface.name.startsWith('lagg')
    && !inOtherBridge.has(iface.name)
  ).map((iface) => iface.name);
});

function removeMember(i) {
  configData.value.bridge_members.splice(i, 1);
}

function addLaggPort() {
  configData.value.lagg_ports.push('');
}

const availableLaggPorts = computed(() => {
  const laggName = configIfaceName.value;
  // Collect all interfaces that are already ports of any lagg.
  const inAnyLagg = new Set();
  for (const iface of interfaces.value) {
    if (iface.name === laggName) continue;
    if (!iface.name.startsWith('lagg')) continue;
    // LAGG ports are not exposed as .members in the backend (only bridge members are).
    // So we rely on the current config's lagg_ports to exclude already-selected.
  }
  const alreadyListed = new Set(configData.value?.lagg_ports?.filter(p => p) || []);
  return interfaces.value.filter((iface) =>
    iface.name !== laggName
    && !iface.is_loopback
    && !iface.name.startsWith('lagg')
    && !iface.name.startsWith('bridge')
    && !alreadyListed.has(iface.name)
  ).map((iface) => iface.name);
});

function removeLaggPort(i) {
  configData.value.lagg_ports.splice(i, 1);
}

async function saveConfig() {
  configSaving.value = true;
  try {
    await api.put(`/api/network/interfaces/${configIfaceName.value}`, configData.value);
    toast.toast(t('net.configSaved'));
    configIfaceName.value = null;
    load();
  } catch (err) {
    await alert(t('common.saveFailed', { msg: '' }), err.message || t('common.operationFailed'));
  } finally {
    configSaving.value = false;
  }
}

async function applyConfig(iface) {
  const name = iface.name;
  if (!await confirm(t('net.applyConfig'), t('net.applyConfirmMsg', { name }))) return;
  configApplying.value = true;
  try {
    await api.post(`/api/network/interfaces/${name}/apply`, {});
    toast.toast(t('net.applySuccess', { name }));
    load();
  } catch (err) {
    await alert(t('net.applyFailed'), err.message || t('common.operationFailed'));
  } finally {
    configApplying.value = false;
  }
}

async function destroyIface(iface) {
  const name = iface.name;
  if (!await confirm(t('net.destroy'), t('net.destroyConfirmMsg', { name }))) return;
  try {
    await api.del(`/api/network/interfaces/${name}`);
    toast.toast(t('common.delete') + ': ' + name);
    load();
  } catch (err) {
    await alert(t('net.destroyFailed'), err.message || t('common.operationFailed'));
  }
}

function openCreateDialog() {
  createType.value = 'bridge';
  createNum.value = 0;
  createCustomName.value = '';
  createDialog.value = true;
}

async function doCreate() {
  const sel = ifaceTypes.value.find((tp) => tp.value === createType.value);
  let name;
  if (!sel) return;
  if (sel.value === 'custom') {
    name = createCustomName.value.trim();
  } else {
    name = `${sel.prefix}${createNum.value}`;
  }
  if (!name) return;
  creating.value = true;
  try {
    await api.post('/api/network/interfaces', { name });
    toast.toast(t('net.createInterface') + ': ' + name);
    createDialog.value = false;
    load();
  } catch (err) {
    await alert(t('net.createFailed'), err.message || t('common.operationFailed'));
  } finally {
    creating.value = false;
  }
}

async function setGateway() {
  const result = await formModal(
    t('net.setGatewayTitle'),
    [
      {
        key: 'gateway',
        label: t('net.gatewayIpv4'),
        type: 'text',
        value: gateway.value?.configured || '',
        placeholder: 'e.g. 192.168.1.1',
        tooltip: t('net.gatewayHint'),
      },
      {
        key: 'gateway6',
        label: t('net.gatewayIpv6'),
        type: 'text',
        value: gateway.value?.configured6 || '',
        placeholder: 'e.g. fe80::1%em0',
        tooltip: t('net.gatewayHint6'),
      },
    ],
    {
      submitLabel: t('common.save'),
      submitHandler: async (values) => {
        const payload = {};
        if (values.gateway !== (gateway.value?.configured || '')) {
          payload.gateway = values.gateway || '';
        }
        if (values.gateway6 !== (gateway.value?.configured6 || '')) {
          payload.gateway6 = values.gateway6 || '';
        }
        await api.put('/api/network/gateway', payload);
      },
    },
  );
  if (result) {
    toast.toast(t('net.gatewaySaved'));
    load();
  }
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
      <button class="btn-secondary" @click="openCreateDialog"><i class="fa-solid fa-plus"></i> {{ t('net.createInterface') }}</button>
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
            <span v-if="iface.driver_name" class="text-dim mono" style="font-size:0.85em;">({{ iface.driver_name }})</span>
            <span class="net-iface-name-spacer"></span>
            <span class="badge">{{ linkLabel(iface) }}</span>
          </div>
          <div class="net-iface-body">
            <div v-if="iface.description" class="kv"><span class="kv-key">{{ t('common.description') }}</span><span class="kv-val">{{ iface.description }}</span></div>
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
            <div v-if="iface.members.length" class="kv"><span class="kv-key">{{ t('net.members') }}</span><span class="kv-val">{{ iface.members.length }}</span></div>
            <div v-if="iface.status" class="kv"><span class="kv-key">{{ t('common.status') }}</span><span class="kv-val" style="white-space:pre-line;">{{ iface.status }}</span></div>
          </div>
          <div class="net-iface-footer">
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="showDetail(iface)">{{ t('net.detail') }}</button>
              <button v-if="canConfigure(iface)" class="btn-secondary btn-sm" @click="showConfig(iface)">{{ t('common.config') }}</button>
            </div>
          </div>
        </div>
      </div>
    </template>

    <template v-if="bridges.length">
      <div class="section-title" :style="{ marginTop: physical.length ? '32px' : '' }">{{ t('net.bridge') }}</div>
      <div class="card-grid">
        <div v-for="iface in bridges" :key="iface.name" class="card net-iface">
          <div class="net-iface-header">
            <i :class="['fa-solid', iface.is_loopback ? 'fa-rotate' : 'fa-ethernet', 'net-iface-icon', iface.is_up ? 'up' : 'down']"></i>
            <span class="net-iface-name mono">{{ iface.name }}</span>
            <span v-if="iface.driver_name" class="text-dim mono" style="font-size:0.85em;">({{ iface.driver_name }})</span>
            <span class="net-iface-name-spacer"></span>
            <span class="badge">{{ linkLabel(iface) }}</span>
          </div>
          <div class="net-iface-body">
            <div v-if="iface.description" class="kv"><span class="kv-key">{{ t('common.description') }}</span><span class="kv-val">{{ iface.description }}</span></div>
            <div class="kv"><span class="kv-key">IPv4</span><span class="kv-val">
              <div v-for="ip in iface.ipv4" :key="ip.address" :class="{ 'text-dim': ip.is_alias }">
                {{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}
                <span v-if="ip.is_alias" class="badge">{{ t('net.alias') }}</span>
              </div>
              <span v-if="!iface.ipv4.length" class="text-dim">—</span>
            </span></div>
            <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">{{ iface.mac || '—' }}</span></div>
            <div v-if="iface.groups.length" class="kv"><span class="kv-key">{{ t('net.groups') }}</span><span class="kv-val"><span v-for="g in iface.groups" :key="g" class="badge badge-dim">{{ g }}</span></span></div>
            <div v-if="iface.members.length" class="kv"><span class="kv-key">{{ t('net.members') }}</span><span class="kv-val">{{ iface.members.length }}</span></div>
            <div v-if="iface.status" class="kv"><span class="kv-key">{{ t('common.status') }}</span><span class="kv-val" style="white-space:pre-line;">{{ iface.status }}</span></div>
          </div>
          <div class="net-iface-footer">
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="showDetail(iface)">{{ t('net.detail') }}</button>
              <button v-if="canConfigure(iface)" class="btn-secondary btn-sm" @click="showConfig(iface)">{{ t('common.config') }}</button>
              <button v-if="canDestroy(iface)" class="btn-danger btn-sm" @click="destroyIface(iface)"><i class="fa-solid fa-trash"></i></button>
            </div>
          </div>
        </div>
      </div>
    </template>

    <template v-if="others.length">
      <div class="section-title" :style="{ marginTop: (physical.length || bridges.length) ? '32px' : '' }">{{ t('net.virtual') }}</div>
      <div class="card-grid">
        <div v-for="iface in others" :key="iface.name" class="card net-iface">
          <div class="net-iface-header">
            <i :class="['fa-solid', iface.is_loopback ? 'fa-rotate' : 'fa-ethernet', 'net-iface-icon', iface.is_up ? 'up' : 'down']"></i>
            <span class="net-iface-name mono">{{ iface.name }}</span>
            <span v-if="iface.driver_name" class="text-dim mono" style="font-size:0.85em;">({{ iface.driver_name }})</span>
            <span class="net-iface-name-spacer"></span>
            <span class="badge">{{ linkLabel(iface) }}</span>
          </div>
          <div class="net-iface-body">
            <div v-if="iface.description" class="kv"><span class="kv-key">{{ t('common.description') }}</span><span class="kv-val">{{ iface.description }}</span></div>
            <div class="kv"><span class="kv-key">IPv4</span><span class="kv-val">
              <div v-for="ip in iface.ipv4" :key="ip.address" :class="{ 'text-dim': ip.is_alias }">
                {{ ip.address }}{{ ip.prefix_len != null ? `/${ip.prefix_len}` : '' }}
              </div>
              <span v-if="!iface.ipv4.length" class="text-dim">—</span>
            </span></div>
            <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">{{ iface.mac || '—' }}</span></div>
            <div v-if="iface.groups.length" class="kv"><span class="kv-key">{{ t('net.groups') }}</span><span class="kv-val"><span v-for="g in iface.groups" :key="g" class="badge badge-dim">{{ g }}</span></span></div>
            <div v-if="iface.members.length" class="kv"><span class="kv-key">{{ t('net.members') }}</span><span class="kv-val">{{ iface.members.length }}</span></div>
            <div v-if="iface.status" class="kv"><span class="kv-key">{{ t('common.status') }}</span><span class="kv-val" style="white-space:pre-line;">{{ iface.status }}</span></div>
          </div>
          <div class="net-iface-footer">
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="showDetail(iface)">{{ t('net.detail') }}</button>
              <button v-if="canConfigure(iface)" class="btn-secondary btn-sm" @click="showConfig(iface)">{{ t('common.config') }}</button>
              <button v-if="canDestroy(iface)" class="btn-danger btn-sm" @click="destroyIface(iface)"><i class="fa-solid fa-trash"></i></button>
            </div>
          </div>
        </div>
      </div>
    </template>

    <!-- Gateway -->
    <template v-if="gateway">
      <div class="section-title" style="margin-top:32px;">{{ t('net.defaultGateway') }}</div>
      <div class="card" style="padding:1rem;">
        <div class="kv"><span class="kv-key">{{ t('net.gatewayIpv4') }}</span><span class="kv-val">
          <strong v-if="gateway.gateway" class="mono">{{ gateway.gateway }}</strong>
          <span v-else class="text-dim">{{ t('common.notConfigured') }}</span>
          {{ gateway.interface ? `(${gateway.interface})` : '' }}
        </span></div>
        <div class="kv"><span class="kv-key">{{ t('net.gatewayIpv6') }}</span><span class="kv-val">
          <strong v-if="gateway.gateway6" class="mono">{{ gateway.gateway6 }}</strong>
          <span v-else class="text-dim">{{ t('common.notConfigured') }}</span>
          {{ gateway.interface6 ? `(${gateway.interface6})` : '' }}
        </span></div>
        <div style="margin-top:0.75rem;">
          <button class="btn-secondary btn-sm" @click="setGateway">{{ t('common.config') }}</button>
        </div>
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
      <h3>{{ detailIface.name }}<span v-if="detailIface.driver_name" class="text-dim" style="font-size:0.8em;"> ({{ detailIface.driver_name }})</span> — {{ t('net.interfaceInfo') }}</h3>
      <div class="kv-grid">
        <div v-if="detailIface.description" class="kv"><span class="kv-key">{{ t('common.description') }}</span><span class="kv-val">{{ detailIface.description }}</span></div>
        <div class="kv"><span class="kv-key">{{ t('common.status') }}</span><span class="kv-val">{{ detailIface.is_up ? t('net.linkUp') : t('net.linkDown') }} ({{ detailIface.link_state }})</span></div>
        <div class="kv"><span class="kv-key">{{ t('net.flags') }}</span><span class="kv-val mono">{{ detailIface.flags.join(', ') }}</span></div>
        <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">{{ detailIface.mac || '—' }}</span></div>
        <div class="kv"><span class="kv-key">MTU</span><span class="kv-val">{{ detailIface.mtu }}</span></div>
        <div class="kv"><span class="kv-key">Metric</span><span class="kv-val">{{ detailIface.metric }}</span></div>
        <div v-if="detailIface.groups.length" class="kv"><span class="kv-key">{{ t('net.groups') }}</span><span class="kv-val"><span v-for="g in detailIface.groups" :key="g" class="badge badge-dim">{{ g }}</span></span></div>
        <div v-if="detailIface.status" class="kv"><span class="kv-key">{{ t('common.status') }}</span><span class="kv-val" style="white-space:pre-line;">{{ detailIface.status }}</span></div>
      </div>
      <div v-if="detailIface.members.length" style="margin-top:1rem;">
        <h4>{{ t('net.members') }}</h4>
        <table>
          <tbody>
            <tr v-for="m in detailIface.members" :key="m.name">
              <td class="mono">{{ m.name }}</td>
              <td class="text-dim">{{ m.info }}</td>
            </tr>
          </tbody>
        </table>
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
        <div class="btn-group">
          <button v-if="canConfigure(detailIface)" class="btn-secondary btn-sm" @click="showConfig(detailIface)">{{ t('common.config') }}</button>
          <button v-if="canDestroy(detailIface)" class="btn-danger btn-sm" @click="destroyIface(detailIface)"><i class="fa-solid fa-trash"></i> {{ t('net.destroy') }}</button>
          <button class="btn-secondary" @click="detailIface = null">{{ t('common.close') }}</button>
        </div>
      </div>
    </div>
  </div>

  <!-- rc.conf config modal -->
  <div v-if="configIfaceName" class="modal-overlay">
    <div class="modal" style="max-width:680px;">
      <h3>{{ configIfaceName }}<span v-if="configDriverName" class="text-dim" style="font-size:0.8em;"> ({{ configDriverName }})</span> — {{ t('net.ifaceConfig') }}</h3>
      <div v-if="configLoading" class="text-dim" style="padding:1rem 0;"><span class="spinner"></span> {{ t('common.loading') }}</div>
      <template v-else-if="configData">
        <!-- Interface properties -->
        <div class="config-section">
          <h4>{{ t('common.config') }}</h4>
          <div class="config-grid">
            <div class="kv"><span class="kv-key">{{ t('common.description') }}</span>
              <input type="text" class="input" v-model="configData.description" placeholder="e.g. WAN"></div>
            <div class="kv"><span class="kv-key">MTU</span>
              <input type="number" class="input" v-model.number="configData.mtu" placeholder="e.g. 1500"></div>
            <div class="kv" style="grid-column:1/-1;"><span class="kv-key">{{ t('net.extraOptions') }}</span>
              <input type="text" class="input mono" v-model="configData.options" :placeholder="t('net.extraOptionsPh')"></div>
          </div>
        </div>

        <!-- UP toggle -->
        <div style="margin-bottom:1rem;">
          <label class="checkbox-row">
            <input type="checkbox" v-model="configData.is_up">
            <span>UP</span>
          </label>
        </div>

        <!-- IPv4 config -->
        <div class="config-section">
          <h4>{{ t('net.ipv4Config') }}</h4>
          <div class="radio-pill-group" style="margin-bottom:0.5rem;">
            <label class="radio-pill" :class="{ active: ipv4Mode === 'none' }">
              <input type="radio" value="none" v-model="ipv4Mode"><span>{{ t('common.none') }}</span>
            </label>
            <label class="radio-pill" :class="{ active: ipv4Mode === 'dhcp' }">
              <input type="radio" value="dhcp" v-model="ipv4Mode"><span>DHCP</span>
            </label>
            <label class="radio-pill" :class="{ active: ipv4Mode === 'static' }">
              <input type="radio" value="static" v-model="ipv4Mode"><span>{{ t('net.staticIp') }}</span>
            </label>
          </div>
          <template v-if="ipv4Mode === 'static'">
            <table class="form-table">
              <thead><tr>
                <th>{{ t('net.ipAddress') }}</th>
                <th>{{ t('net.netmask') }}</th>
                <th style="width:40px;"></th>
              </tr></thead>
              <tbody><tr>
                <td><input type="text" class="input mono" v-model="configData.ipv4" placeholder="e.g. 192.168.1.10"></td>
                <td><input type="text" class="input mono" v-model="configData.ipv4_netmask" placeholder="255.255.255.0"></td>
                <td></td>
              </tr></tbody>
            </table>
          </template>
        </div>

        <!-- IPv4 aliases -->
        <div class="config-section">
          <h4>{{ t('net.ipv4Aliases') }}</h4>
          <table v-if="configData.ipv4_aliases.length" class="form-table">
            <thead><tr>
              <th>{{ t('net.ipAddress') }}</th>
              <th>{{ t('net.netmask') }}</th>
              <th style="width:40px;"></th>
            </tr></thead>
            <tbody>
              <tr v-for="(a, i) in configData.ipv4_aliases" :key="'alias-'+i">
                <td><input type="text" class="input mono" v-model="a.address" placeholder="192.168.1.11"></td>
                <td><input type="text" class="input mono" v-model="a.netmask" placeholder="255.255.255.0"></td>
                <td><button class="btn-secondary btn-sm" @click="removeAlias(i)"><i class="fa-solid fa-xmark"></i></button></td>
              </tr>
            </tbody>
          </table>
          <button class="btn-secondary btn-sm" @click="addAlias"><i class="fa-solid fa-plus"></i> {{ t('net.addAlias') }}</button>
        </div>

        <!-- IPv6 config -->
        <div class="config-section">
          <h4>{{ t('net.ipv6Config') }}</h4>
          <div class="radio-pill-group" style="margin-bottom:0.5rem;">
            <label class="radio-pill" :class="{ active: ipv6Mode === 'none' }">
              <input type="radio" value="none" v-model="ipv6Mode"><span>{{ t('common.none') }}</span>
            </label>
            <label class="radio-pill" :class="{ active: ipv6Mode === 'slaac' }">
              <input type="radio" value="slaac" v-model="ipv6Mode"><span>SLAAC</span>
              <FieldHelp :text="t('net.ipv6SlaacDesc')" />
            </label>
            <label class="radio-pill" :class="{ active: ipv6Mode === 'static' }">
              <input type="radio" value="static" v-model="ipv6Mode"><span>{{ t('net.staticIp') }}</span>
            </label>
          </div>
          <template v-if="ipv6Mode === 'static'">
            <table class="form-table">
              <thead><tr>
                <th>{{ t('net.ipAddress') }}</th>
                <th style="width:120px;">{{ t('net.prefixLen') }}</th>
                <th style="width:40px;"></th>
              </tr></thead>
              <tbody>
                <tr v-for="(e, i) in configData.ipv6" :key="'ipv6-'+i">
                  <td><input type="text" class="input mono" v-model="e.address" placeholder="2001:db8::1"></td>
                  <td><input type="text" class="input mono" v-model="e.prefixlen" placeholder="64"></td>
                  <td><button class="btn-secondary btn-sm" @click="removeIpv6(i)"><i class="fa-solid fa-xmark"></i></button></td>
                </tr>
              </tbody>
            </table>
            <button class="btn-secondary btn-sm" @click="addIpv6"><i class="fa-solid fa-plus"></i> {{ t('net.addIpv6') }}</button>
          </template>
        </div>

        <!-- LAGG config -->
        <div v-if="configData.is_lagg" class="config-section">
          <h4>{{ t('net.laggConfig') }}</h4>
          <div class="kv" style="margin-bottom:8px;"><span class="kv-key">{{ t('net.laggProto') }}</span>
            <select v-model="configData.lagg_proto" class="input">
              <option value=""></option>
              <option value="lacp">lacp</option>
              <option value="loadbalance">loadbalance</option>
              <option value="roundrobin">roundrobin</option>
              <option value="failover">failover</option>
              <option value="fec">fec</option>
              <option value="none">none</option>
            </select>
          </div>
          <div class="kv" style="margin-bottom:4px;"><span class="kv-key">{{ t('net.laggPorts') }}</span></div>
          <div v-for="(p, i) in configData.lagg_ports" :key="'lagg-'+i" class="form-row">
            <select class="input mono" v-model="configData.lagg_ports[i]">
              <option value="">— select —</option>
              <option v-for="name in availableLaggPorts" :key="name" :value="name">{{ name }}</option>
            </select>
            <button class="btn-secondary btn-sm" @click="removeLaggPort(i)"><i class="fa-solid fa-xmark"></i></button>
          </div>
          <button v-if="availableLaggPorts.length" class="btn-secondary btn-sm" @click="addLaggPort"><i class="fa-solid fa-plus"></i> {{ t('net.addLaggPort') }}</button>
        </div>

        <!-- Bridge members -->
        <div v-if="configData.is_bridge" class="config-section">
          <h4>{{ t('net.bridgeMembers') }}</h4>
          <div v-for="(m, i) in configData.bridge_members" :key="'bm-'+i" class="form-row">
            <select class="input mono" v-model="configData.bridge_members[i]">
              <option value="">— select —</option>
              <option v-for="name in availableBridgeMembers" :key="name" :value="name">{{ name }}</option>
            </select>
            <button class="btn-secondary btn-sm" @click="removeMember(i)"><i class="fa-solid fa-xmark"></i></button>
          </div>
          <button v-if="availableBridgeMembers.length" class="btn-secondary btn-sm" @click="addMember"><i class="fa-solid fa-plus"></i> {{ t('net.addMember') }}</button>
        </div>

        <div class="modal-actions">
          <div class="btn-group">
            <button class="btn-secondary" @click="configIfaceName = null">{{ t('common.cancel') }}</button>
            <button @click="saveConfig" :disabled="configSaving">{{ t('common.save') }}</button>
          </div>
        </div>
      </template>
    </div>
  </div>

  <!-- Create interface dialog -->
  <div v-if="createDialog" class="modal-overlay">
    <div class="modal" style="max-width:480px;">
      <h3>{{ t('net.createInterface') }}</h3>
      <p class="text-dim" style="margin-bottom:1rem;">{{ t('net.createInterfaceDesc') }}</p>
      <div style="display:flex; gap:12px; margin-bottom:1rem;">
        <div style="flex:1;">
          <label>{{ t('common.type') }}</label>
          <select class="input" v-model="createType">
            <option v-for="tp in ifaceTypes" :key="tp.value" :value="tp.value">{{ tp.label }}</option>
          </select>
        </div>
        <div v-if="createType !== 'custom'" style="width:100px;">
          <label>{{ t('net.ifaceNumber') }}</label>
          <input type="number" class="input" v-model.number="createNum" min="0" @keyup.enter="doCreate">
        </div>
        <div v-else style="flex:1;">
          <label>{{ t('common.name') }}</label>
          <input type="text" class="input mono" v-model="createCustomName" :placeholder="t('net.ifaceNamePlaceholder')" @keyup.enter="doCreate">
        </div>
      </div>
      <div v-if="createPreview" style="margin-bottom:1rem;">
        <label>{{ t('net.ifaceNamePreview') }}</label>
        <div class="mono" style="font-size:16px; padding:8px 12px; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius);">{{ createPreview }}</div>
      </div>
      <div class="modal-actions">
        <div class="btn-group">
          <button class="btn-secondary" @click="createDialog = false">{{ t('common.cancel') }}</button>
          <button @click="doCreate" :disabled="creating || !createPreview">{{ t('common.create') }}</button>
        </div>
      </div>
    </div>
  </div>
</template>
