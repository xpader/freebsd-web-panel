<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const name = route.params.name;

const d = ref(null);
const error = ref('');
const acting = ref(false);

const isRunning = computed(() => {
  if (!d.value) return false;
  return d.value.state === 'running' || d.value.state.startsWith('bootloader') || d.value.state.startsWith('running');
});

const isLocked = computed(() => d.value?.state === 'locked');

function stateBadge() {
  if (!d.value) return { cls: '', text: '' };
  const st = d.value.state;
  if (st === 'running' || st.startsWith('running') || st.startsWith('bootloader'))
    return { cls: 'badge-success', text: t('bhyve.stateRunning') };
  if (st === 'locked') return { cls: 'badge-warn', text: t('bhyve.stateLocked') };
  if (st === 'suspended') return { cls: 'badge-dim', text: t('bhyve.stateSuspended') };
  return { cls: 'badge-dim', text: t('bhyve.stateStopped') };
}

const VNC_KEYS = new Set([
  'graphics', 'graphics_port', 'graphics_res', 'graphics_wait', 'xhci_mouse',
]);

const configEntries = computed(() => {
  if (!d.value?.config) return [];
  return Object.entries(d.value.config)
    .filter(([k]) => !VNC_KEYS.has(k))
    .map(([k, v]) => ({ key: k, value: v }));
});

const vncEntries = computed(() => {
  if (!d.value?.config) return [];
  return Object.entries(d.value.config)
    .filter(([k]) => VNC_KEYS.has(k))
    .map(([k, v]) => ({ key: k, value: v }));
});

async function reload() {
  error.value = '';
  try {
    d.value = await api.get(`/api/bhyve/vms/${encodeURIComponent(name)}`);
  } catch (err) {
    error.value = err.message || '';
  }
}

async function vmAction(action) {
  acting.value = true;
  try {
    await api.post(`/api/bhyve/vms/${encodeURIComponent(name)}/${action}`);
    toast.toast(action === 'start'
      ? t('bhyve.startedToast', { name })
      : t('bhyve.stoppedToast', { name }));
    await new Promise((r) => setTimeout(r, 1000));
    await reload();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    acting.value = false;
  }
}

onMounted(reload);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/bhyve/vms" />
      <h1>{{ name }}</h1>
      <span v-if="d" :class="['badge', stateBadge().cls]" style="margin-left:8px;">{{ stateBadge().text }}</span>
    </div>
    <div v-if="d" class="flex btn-group" style="margin-left:auto;">
      <button v-if="!isRunning" class="btn-sm" :disabled="acting || isLocked" @click="vmAction('start')">
        <i class="fa-solid fa-play"></i> {{ t('common.start') }}
      </button>
      <button v-if="isRunning" class="btn-secondary btn-sm" :disabled="acting" @click="vmAction('stop')">
        <i class="fa-solid fa-stop"></i> {{ t('common.stop') }}
      </button>
      <a v-if="isRunning" :href="`#/bhyve/console/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-terminal"></i> {{ t('common.console') }}</a>
      <a v-if="d.vnc_port && isRunning" :href="`#/bhyve/vnc/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-display"></i> VNC</a>
      <button class="btn-secondary btn-sm" :disabled="acting" @click="router.push(`/bhyve/edit/${name}`)"><i class="fa-solid fa-pen"></i> {{ t('common.edit') }}</button>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!d" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <!-- Overview -->
    <div class="card">
      <h3>{{ t('common.overview') }}</h3>
      <div class="stat-row" style="flex-wrap:wrap;">
        <span>UUID: <strong class="mono">{{ d.uuid }}</strong></span>
        <span>{{ t('bhyve.loader') }}: <strong>{{ d.loader }}</strong></span>
        <span>{{ t('bhyve.datastore') }}: <strong>{{ d.datastore }}</strong></span>
        <span>CPU: <strong>{{ d.cpu }}</strong></span>
        <span>{{ t('bhyve.memory') }}: <strong>{{ d.memory }}</strong></span>
        <span v-if="d.memory_resident">{{ t('bhyve.memoryResident') }}: <strong class="mono">{{ d.memory_resident }}</strong></span>
        <span v-if="d.console_port">{{ t('common.console') }}: <strong class="mono">{{ d.console_port }}</strong></span>
      </div>
    </div>

    <!-- Disks -->
    <div class="card" v-if="d.disks.length">
      <h3>{{ t('common.disks') }}</h3>
      <table>
        <thead><tr>
          <th>#</th>
          <th>{{ t('bhyve.deviceType') }}</th>
          <th>{{ t('bhyve.emulation') }}</th>
          <th>{{ t('bhyve.systemPath') }}</th>
          <th>{{ t('common.size') }}</th>
          <th>{{ t('common.used') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="disk in d.disks" :key="disk.number">
            <td class="mono">{{ disk.number }}</td>
            <td>{{ disk.device_type }}</td>
            <td class="mono">{{ disk.emulation }}</td>
            <td class="mono">{{ disk.system_path }}</td>
            <td class="mono">{{ disk.bytes_size || '—' }}</td>
            <td class="mono">{{ disk.bytes_used || '—' }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Network -->
    <div class="card" v-if="d.networks.length">
      <h3>{{ t('common.network') }}</h3>
      <table>
        <thead><tr>
          <th>#</th>
          <th>{{ t('bhyve.emulation') }}</th>
          <th>{{ t('bhyve.virtualSwitch') }}</th>
          <th>MAC</th>
          <th v-if="d.networks.some(n => n.active_device)">{{ t('bhyve.activeDevice') }}</th>
          <th v-if="d.networks.some(n => n.bytes_in)">RX</th>
          <th v-if="d.networks.some(n => n.bytes_out)">TX</th>
        </tr></thead>
        <tbody>
          <tr v-for="net in d.networks" :key="net.number">
            <td class="mono">{{ net.number }}</td>
            <td class="mono">{{ net.emulation }}</td>
            <td>{{ net.virtual_switch || '—' }}</td>
            <td class="mono">{{ net.mac_address || '—' }}</td>
            <td v-if="d.networks.some(n => n.active_device)" class="mono">{{ net.active_device || '—' }}</td>
            <td v-if="d.networks.some(n => n.bytes_in)" class="mono">{{ net.bytes_in || '—' }}</td>
            <td v-if="d.networks.some(n => n.bytes_out)" class="mono">{{ net.bytes_out || '—' }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Snapshots -->
    <div class="card" v-if="d.snapshots.length">
      <h3>{{ t('common.snapshots') }}</h3>
      <table>
        <thead><tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('common.size') }}</th>
          <th>{{ t('common.createdAt') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="snap in d.snapshots" :key="snap.name">
            <td class="mono"><strong>{{ snap.name }}</strong></td>
            <td class="mono">{{ snap.size }}</td>
            <td class="mono">{{ snap.date }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- VNC / Graphics -->
    <div class="card" v-if="vncEntries.length">
      <h3>{{ t('bhyve.vncInfo') }}</h3>
      <table>
        <thead><tr>
          <th>{{ t('common.key') }}</th>
          <th>{{ t('common.value') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="e in vncEntries" :key="e.key">
            <td class="mono">{{ e.key }}</td>
            <td class="mono">{{ e.value }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Config (.conf key-values) -->
    <div class="card" v-if="configEntries.length">
      <h3>{{ t('bhyve.configFile') }}</h3>
      <table>
        <thead><tr>
          <th>{{ t('common.key') }}</th>
          <th>{{ t('common.value') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="e in configEntries" :key="e.key">
            <td class="mono">{{ e.key }}</td>
            <td class="mono">{{ e.value }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>

</template>
