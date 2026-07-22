<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';
import { fmtBytesStr, fmtUptime } from '../lib/format.js';
import { pollUntil } from '../lib/poll.js';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const name = route.params.name;

const d = ref(null);
const error = ref('');
const acting = ref(false);
const transitioning = ref('');  // '' | 'start' | 'stop'
const refreshing = ref(false);

const isRunning = computed(() => {
  if (!d.value) return false;
  return d.value.state === 'running' || d.value.state.startsWith('bootloader') || d.value.state.startsWith('running');
});

const isLocked = computed(() => {
  if (!d.value) return false;
  return d.value.state.toLowerCase().startsWith('locked');
});

function stateBadge() {
  if (!d.value) return { cls: '', text: '' };
  if (transitioning.value === 'start')
    return { cls: 'badge-warn', text: t('bhyve.starting') };
  if (transitioning.value === 'stop')
    return { cls: 'badge-warn', text: t('bhyve.stopping') };
  const st = d.value.state.toLowerCase();
  if (st === 'running' || st.startsWith('running') || st.startsWith('bootloader'))
    return { cls: 'badge-success', text: t('bhyve.stateRunning') };
  if (st.startsWith('locked')) return { cls: 'badge-warn', text: t('bhyve.stateLocked') };
  if (st === 'suspended') return { cls: 'badge-dim', text: t('bhyve.stateSuspended') };
  return { cls: 'badge-dim', text: t('bhyve.stateStopped') };
}

/* ── label maps (matching BhyveEditPage) ── */

const diskTypeLabels = { 'virtio-blk': 'VirtIO', 'ahci-hd': 'bhyve.diskTypeAhciHd', 'ahci-cd': 'bhyve.diskTypeAhciCd', 'nvme': 'NVMe', 'virtio-9p': 'bhyve.diskTypeVirtio9p' };
const diskDevLabels = { 'file': 'bhyve.diskDevFile', 'zvol': 'ZVol', 'custom': 'bhyve.diskDevCustom', 'iscsi': 'iSCSI' };
const networkTypeLabels = { 'virtio-net': 'VirtIO', 'e1000': 'E1000' };

function diskTypeLabel(value) {
  const label = diskTypeLabels[value];
  if (!label) return value;
  return label.startsWith('bhyve.') ? t(label) : label;
}
function diskDevLabel(value) {
  const label = diskDevLabels[value];
  if (!label) return value;
  return label.startsWith('bhyve.') ? t(label) : label;
}
function networkTypeLabel(value) {
  return networkTypeLabels[value] || value;
}

const GRAPHICS_KEYS = [
  'graphics', 'graphics_port', 'graphics_listen', 'graphics_res',
  'graphics_wait', 'graphics_vga', 'vnc_password', 'xhci_mouse',
];

const graphicsFieldLabels = {
  graphics: 'bhyve.fieldGraphicsEnabled',
  graphics_port: 'bhyve.fieldGraphicsPort',
  graphics_listen: 'bhyve.fieldGraphicsListen',
  graphics_res: 'bhyve.fieldGraphicsResolution',
  graphics_wait: 'bhyve.fieldGraphicsWait',
  graphics_vga: 'bhyve.fieldGraphicsVga',
  vnc_password: 'bhyve.fieldVncPassword',
  xhci_mouse: 'bhyve.fieldXhciMouse',
};

/* ── computed display data ── */

const parsedDisks = computed(() => {
  if (!d.value?.config) return [];
  const cfg = d.value.config;
  const byNum = {};
  if (Array.isArray(d.value.disks)) {
    for (const disk of d.value.disks) byNum[disk.number] = disk;
  }
  const indexes = [...new Set(Object.keys(cfg)
    .map((k) => k.match(/^disk(\d+)_/)?.[1])
    .filter(Boolean))]
    .sort((a, b) => Number(a) - Number(b));
  return indexes.map((i) => {
    const idx = Number(i);
    const type = cfg[`disk${idx}_type`] || 'virtio-blk';
    const rawDev = cfg[`disk${idx}_dev`] || 'file';
    const dev = rawDev === 'sparse-zvol' ? 'zvol' : rawDev;
    const rawName = cfg[`disk${idx}_name`] || '';
    const opts = cfg[`disk${idx}_opts`] || '';
    const info = byNum[idx] || {};
    const size = info.bytes_size ? fmtBytesStr(info.bytes_size) : '';
    const used = info.bytes_used ? fmtBytesStr(info.bytes_used) : '';
    let display;
    if (type === 'virtio-9p') {
      const sep = rawName.indexOf('=');
      display = sep >= 0 ? `${rawName.slice(0, sep)} → ${rawName.slice(sep + 1)}` : rawName;
    } else {
      display = rawName;
    }
    return { index: idx, type, dev, display, opts, size, used };
  });
});

const parsedNetworks = computed(() => {
  if (!d.value?.config) return [];
  const cfg = d.value.config;
  const byNum = {};
  if (Array.isArray(d.value.networks)) {
    for (const net of d.value.networks) byNum[net.number] = net;
  }
  const indexes = [...new Set(Object.keys(cfg)
    .map((k) => k.match(/^network(\d+)_/)?.[1])
    .filter(Boolean))]
    .sort((a, b) => Number(a) - Number(b));
  return indexes.map((i) => {
    const idx = Number(i);
    const type = cfg[`network${idx}_type`] || 'virtio-net';
    const sw = cfg[`network${idx}_switch`] || '';
    const mac = cfg[`network${idx}_mac`] || '';
    const info = byNum[idx] || {};
    return {
      index: idx, type, switchName: sw, mac,
      activeDevice: info.active_device || '',
      rx: info.bytes_in || '',
      tx: info.bytes_out || '',
    };
  });
});

const hasNetRuntime = computed(() => parsedNetworks.value.some((n) => n.activeDevice || n.rx || n.tx));

const graphicsEntries = computed(() => {
  if (!d.value?.config) return [];
  return GRAPHICS_KEYS
    .filter((k) => k in d.value.config)
    .map((k) => ({
      key: k,
      label: t(graphicsFieldLabels[k]),
      value: k === 'vnc_password' ? '••••••' : d.value.config[k],
    }));
});


async function reload() {
  refreshing.value = true;
  error.value = '';
  try {
    d.value = await api.get(`/api/bhyve/vms/${encodeURIComponent(name)}`);
  } catch (err) {
    error.value = err.message || '';
  } finally {
    refreshing.value = false;
  }
}

async function vmAction(action) {
  acting.value = true;
  transitioning.value = action === 'start' ? 'start' : 'stop';
  try {
    await api.post(`/api/bhyve/vms/${encodeURIComponent(name)}/${action}`);
    toast.toast(action === 'start'
      ? t('bhyve.startedToast', { name })
      : t('bhyve.stoppedToast', { name }));
  } catch (e) {
    acting.value = false;
    transitioning.value = '';
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  // Poll the lightweight state endpoint until target state (or timeout).
  await pollUntil(async () => {
    try {
      const st = await api.get(`/api/bhyve/vms/${encodeURIComponent(name)}/state`);
      if (d.value) d.value.state = st.state;
      return action === 'start'
        ? st.state === 'running' || st.state === 'bootloader'
        : st.state === 'stopped';
    } catch { return false; }
  });
  // Final full reload to get fresh runtime data (uptime, memory, network stats).
  await reload();
  acting.value = false;
  transitioning.value = '';
}

async function poweroffVm() {
  if (!await confirm(t('bhyve.poweroff'), t('bhyve.poweroffConfirm', { name }))) return;
  acting.value = true;
  transitioning.value = 'stop';
  try {
    await api.post(`/api/bhyve/vms/${encodeURIComponent(name)}/poweroff`);
    toast.toast(t('bhyve.poweroffToast', { name }));
  } catch (e) {
    acting.value = false;
    transitioning.value = '';
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  // Poll the lightweight state endpoint until stopped (or timeout).
  await pollUntil(async () => {
    try {
      const st = await api.get(`/api/bhyve/vms/${encodeURIComponent(name)}/state`);
      if (d.value) d.value.state = st.state;
      return st.state === 'stopped';
    } catch { return false; }
  });
  await reload();
  acting.value = false;
  transitioning.value = '';
}

async function destroyVm() {
  if (!await confirm(t('bhyve.destroyVm'), t('bhyve.destroyVmConfirm', { name }))) return;
  acting.value = true;
  try {
    await api.del(`/api/bhyve/vms/${encodeURIComponent(name)}`);
    toast.toast(t('bhyve.destroyedToast', { name }));
    router.push('/bhyve/vms');
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
    </div>
    <div v-if="d" class="flex btn-group" style="margin-left:auto;">
      <button v-if="!isRunning" class="btn-sm" :disabled="acting" @click="vmAction('start')">
        <i class="fa-solid fa-play"></i> {{ t('common.start') }}
      </button>
      <button v-if="isRunning" class="btn-secondary btn-sm" :disabled="acting" @click="vmAction('stop')">
        <i class="fa-solid fa-stop"></i> {{ t('common.stop') }}
      </button>
      <button v-if="isRunning" class="btn-secondary btn-sm" :disabled="acting" @click="poweroffVm">
        <i class="fa-solid fa-plug-circle-xmark"></i> {{ t('bhyve.poweroff') }}
      </button>
      <a v-if="isRunning" :href="`#/bhyve/console/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-terminal"></i> {{ t('common.console') }}</a>
      <a v-if="d.vnc_port && isRunning" :href="`#/bhyve/vnc/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-display"></i> VNC</a>
      <a :href="`#/bhyve/edit/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-pen-to-square"></i> {{ t('common.edit') }}</a>
      <button v-if="!isRunning" class="btn-sm btn-danger" :disabled="acting" @click="destroyVm">
        <i class="fa-solid fa-trash"></i> {{ t('bhyve.destroyVm') }}
      </button>
      <button class="btn-secondary btn-sm" :disabled="acting" @click="reload">
        <i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}
      </button>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!d" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <!-- Status bar -->
    <div class="card">
      <div class="flex" style="flex-wrap:wrap;gap:16px;align-items:center;">
        <div class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('common.status') }}</span>
          <span :class="['badge', stateBadge().cls]">{{ stateBadge().text }}</span>
        </div>
        <div v-if="d.memory_resident" class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">{{ t('bhyve.memoryResident') }}</span><strong class="mono">{{ fmtBytesStr(d.memory_resident) }}</strong></div>
        <div v-if="d.uptime" class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">{{ t('dash.uptime') }}</span><strong class="mono">{{ fmtUptime(d.uptime) }}</strong></div>
        <div v-if="d.vnc_port" class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">VNC</span><strong class="mono">:{{ d.vnc_port }}</strong></div>
        <div v-if="d.console_port" class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">{{ t('common.console') }}</span><strong class="mono">{{ d.console_port }}</strong></div>
      </div>
    </div>

    <!-- Basic info -->
    <div class="card">
      <h3>{{ t('jails.basicInfo') }}</h3>
      <table class="kv-table four-col">
        <tbody>
        <tr>
          <td>UUID</td><td class="mono">{{ d.uuid || '—' }}</td>
          <td>{{ t('bhyve.loader') }}</td><td>{{ d.loader }}</td>
        </tr>
        <tr>
          <td>{{ t('bhyve.datastore') }}</td><td>{{ d.datastore }}</td>
          <td>{{ t('bhyve.cpuCores') }}</td><td>{{ d.cpu }}</td>
        </tr>
        <tr>
          <td>{{ t('bhyve.autoStartBoot') }}</td>
          <td>
            <span v-if="d.auto_start" class="badge badge-success">{{ t('common.enabled') }}</span>
            <span v-else class="badge badge-dim">{{ t('common.disabled') }}</span>
          </td>
          <td>{{ t('bhyve.memory') }}</td><td class="mono">{{ d.memory }}</td>
        </tr>
        </tbody>
      </table>
    </div>

    <!-- Disks -->
    <div class="card" v-if="parsedDisks.length">
      <h3>{{ t('common.disks') }}</h3>
      <table>
        <thead><tr>
          <th>#</th>
          <th>{{ t('bhyve.fieldDeviceType') }}</th>
          <th>{{ t('bhyve.fieldDiskBackend') }}</th>
          <th>{{ t('bhyve.fieldDeviceName') }}</th>
          <th>{{ t('common.size') }}</th>
          <th>{{ t('common.used') }}</th>
          <th>{{ t('bhyve.fieldDeviceOptions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="disk in parsedDisks" :key="disk.index">
            <td class="mono">{{ disk.index }}</td>
            <td>{{ diskTypeLabel(disk.type) }}</td>
            <td>{{ disk.type === 'virtio-9p' ? '—' : diskDevLabel(disk.dev) }}</td>
            <td class="mono">{{ disk.display || '—' }}</td>
            <td class="mono">{{ disk.size || '—' }}</td>
            <td class="mono">{{ disk.used || '—' }}</td>
            <td class="mono">{{ disk.opts || '—' }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Network -->
    <div class="card" v-if="parsedNetworks.length">
      <h3>{{ t('common.network') }}</h3>
      <table>
        <thead><tr>
          <th>#</th>
          <th>{{ t('bhyve.fieldNetworkAdapter') }}</th>
          <th>{{ t('bhyve.fieldSwitch') }}</th>
          <th>MAC</th>
          <th v-if="hasNetRuntime">{{ t('bhyve.activeDevice') }}</th>
          <th v-if="parsedNetworks.some(n => n.rx)">RX</th>
          <th v-if="parsedNetworks.some(n => n.tx)">TX</th>
        </tr></thead>
        <tbody>
          <tr v-for="net in parsedNetworks" :key="net.index">
            <td class="mono">{{ net.index }}</td>
            <td>{{ networkTypeLabel(net.type) }}</td>
            <td>{{ net.switchName || '—' }}</td>
            <td class="mono">{{ net.mac || '—' }}</td>
            <td v-if="hasNetRuntime" class="mono">{{ net.activeDevice || '—' }}</td>
            <td v-if="parsedNetworks.some(n => n.rx)" class="mono">{{ net.rx || '—' }}</td>
            <td v-if="parsedNetworks.some(n => n.tx)" class="mono">{{ net.tx || '—' }}</td>
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
    <div class="card" v-if="graphicsEntries.length">
      <h3>{{ t('bhyve.vncInfo') }}</h3>
      <table>
        <thead><tr>
          <th>{{ t('common.key') }}</th>
          <th>{{ t('common.value') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="e in graphicsEntries" :key="e.key">
            <td>{{ e.label }}</td>
            <td class="mono">{{ e.value }}</td>
          </tr>
        </tbody>
      </table>
    </div>

  </template>

</template>
