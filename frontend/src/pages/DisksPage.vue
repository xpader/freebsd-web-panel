<script setup>
import { ref, reactive, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import ProgressBar from '../components/ui/ProgressBar.vue';
import TaskConsole from '../components/ui/TaskConsole.vue';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();

const disks = ref(null);
const error = ref('');
// name -> bool: inline detail panel (params + partition table) expanded.
const expanded = reactive({});
// Disk name whose SMART modal is currently open, or null.
const smartDialog = ref(null);
// name -> { state: 'loading'|'error'|'ok', data, error }
const smart = reactive({});

function usedBytes(d) {
  return d.partitions.reduce((s, p) => s + p.mediasize_bytes, 0);
}

async function copyUuid(uuid) {
  try {
    await navigator.clipboard.writeText(uuid);
    toast.toast(t('disks.uuidCopied'));
  } catch {
    await alert(t('common.operationFailed'), t('disks.copyFailed'));
  }
}

// Load SMART on demand (when the user opens the modal), not for every disk on
// mount — each call spawns `smartctl` once on the backend.
async function loadSmart(name) {
  smart[name] = { state: 'loading', data: null, error: '' };
  try {
    smart[name] = {
      state: 'ok',
      data: await api.get(`/api/filesystem/disks/${encodeURIComponent(name)}/smart`),
      error: '',
    };
  } catch (err) {
    smart[name] = { state: 'error', data: null, error: err.message || '' };
  }
}

function openSmart(name) {
  smartDialog.value = name;
  if (!smart[name]) loadSmart(name);
}

function closeSmart() {
  smartDialog.value = null;
}
const installTaskId = ref('');
const installing = ref(false);

// Install smartmontools via pkg (background task with streaming output). On
// success, reload SMART data for the disk whose dialog is still open.
async function installSmartmontools() {
  installing.value = true;
  try {
    const res = await api.post('/api/pkg/install', { packages: ['smartmontools'] });
    installTaskId.value = res.task_id;
  } catch (e) {
    installing.value = false;
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

function onInstallDone({ success }) {
  installing.value = false;
  installTaskId.value = '';
  const name = smartDialog.value;
  if (success) {
    toast.toast(t('pkg.installDone', { name: 'smartmontools' }));
    if (name) {
      delete smart[name];
      loadSmart(name);
    }
  } else {
    alert(t('common.operationFailed'), t('pkg.installFailed', { name: 'smartmontools' }));
  }
}

function onKeydown(e) {
  if (e.key === 'Escape' && smartDialog.value) closeSmart();
}

onMounted(async () => {
  try {
    disks.value = await api.get('/api/filesystem/disks');
  } catch (err) {
    error.value = err.message || '';
  }
  window.addEventListener('keydown', onKeydown);
});
onUnmounted(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <div class="page-header">
    <h1>{{ t('disks.title') }}</h1>
    <p>{{ t('disks.subtitle') }}</p>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!disks" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  <div v-else-if="!disks.length" class="card empty">{{ t('fs.noDisks') }}</div>

  <template v-else>
    <div v-for="d in disks" :key="d.name" class="card" style="padding:0;">
      <div class="flex" style="justify-content:space-between;align-items:center;padding:14px 18px;gap:12px;">
        <div class="flex" style="align-items:center;gap:8px;min-width:0;">
          <i class="fa-solid fa-hard-drive" style="font-size:20px;color:var(--accent);flex-shrink:0;"></i>
          <span class="mono" style="font-size:18px;font-weight:700;">{{ d.name }}</span>
          <span class="text-dim">·</span>
          <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">{{ d.descr || '—' }}</span>
        </div>
        <div class="flex" style="align-items:center;gap:8px;flex-shrink:0;">
          <span v-if="d.scheme" class="badge badge-dim">{{ d.scheme }}</span>
          <span v-else class="badge badge-dim">{{ t('disks.noPartitionTable') }}</span>
          <span v-if="d.state" :class="['badge', d.state === 'OK' ? 'badge-success' : 'badge-warn']">{{ d.state }}</span>
          <span class="text-dim mono" style="font-size:13px;">{{ fmtBytes(d.size_bytes) }}</span>
          <div class="btn-group">
            <button class="btn btn-sm" @click="expanded[d.name] = !expanded[d.name]">{{ t('disks.detail') }}</button>
            <button class="btn btn-sm" @click="openSmart(d.name)">SMART</button>
          </div>
        </div>
      </div>

      <div class="stat-grid" style="margin:16px 18px 0;">
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.devicePath') }}</div><div class="mono" style="font-size:13px;word-break:break-all;">/dev/{{ d.name }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.serialIdent') }}</div><div class="mono" style="font-size:13px;">{{ d.ident || '—' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.sectorSize') }}</div><div class="mono" style="font-size:13px;">{{ d.sectorsize ? d.sectorsize + ' B' : '—' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.rpm') }}</div><div style="font-size:13px;">{{ d.rotation_rate === 'unknown' ? t('fs.ssdUnknown') : d.rotation_rate + ' rpm' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.accessMode') }}</div><div class="mono" style="font-size:13px;">{{ d.mode || '—' }}</div></div>
      </div>

      <div style="padding:0 18px 16px;">
        <div class="flex" style="justify-content:space-between;font-size:12px;margin-bottom:6px;">
          <span class="text-dim">{{ t('disks.allocated', { used: fmtBytes(usedBytes(d)), free: fmtBytes(Math.max(0, d.size_bytes - usedBytes(d))) }) }}</span>
          <span class="mono text-dim">{{ (usedBytes(d) / d.size_bytes * 100).toFixed(0) }}%</span>
        </div>
        <ProgressBar :pct="usedBytes(d) / d.size_bytes * 100" variant="auto" />
      </div>

      <!-- Inline detail panel (params + partition table) — toggle via 详情 -->
      <div v-if="expanded[d.name]" style="border-top:1px solid var(--border);">
        <div class="stat-grid" style="margin:16px 18px;">
          <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.model') }}</div><div style="font-size:13px;">{{ d.descr || '—' }}</div></div>
          <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.lunId') }}</div><div class="mono" style="font-size:13px;">{{ d.lunid || '—' }}</div></div>
          <template v-if="d.scheme"><div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.partScheme') }}</div><div style="font-size:13px;">{{ d.scheme }}</div></div></template>
          <template v-if="d.entries != null"><div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.gptEntries') }}</div><div class="mono" style="font-size:13px;">{{ d.entries }}</div></div></template>
          <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.fwSectors') }}</div><div class="mono" style="font-size:13px;">{{ d.fwsectors || '—' }}</div></div>
          <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.fwHeads') }}</div><div class="mono" style="font-size:13px;">{{ d.fwheads || '—' }}</div></div>
        </div>

        <div style="padding:0 18px 18px;">
          <h2 style="font-size:14px;margin:8px 0 8px;">{{ t('disks.partTable', { n: d.partitions.length }) }}</h2>
          <table>
            <thead><tr>
              <th>{{ t('common.device') }}</th><th>{{ t('common.type') }}</th><th>{{ t('disks.label') }}</th>
              <th>{{ t('common.size') }}</th><th>{{ t('disks.startSector') }}</th><th>{{ t('disks.endSector') }}</th><th>{{ t('disks.uuid') }}</th>
            </tr></thead>
            <tbody>
              <tr v-if="!d.partitions.length"><td colspan="7" class="empty">{{ t('disks.noPartitions') }}</td></tr>
              <tr v-for="p in [...d.partitions].sort((a, b) => a.index - b.index)" :key="p.name">
                <td class="mono"><strong>{{ p.name }}</strong></td>
                <td><span class="badge badge-dim">{{ p.type }}</span></td>
                <td>{{ p.label || '—' }}</td>
                <td class="mono">{{ fmtBytes(p.mediasize_bytes) }}</td>
                <td class="mono text-dim">{{ p.start }}</td>
                <td class="mono text-dim">{{ p.end }}</td>
                <td class="mono text-dim" style="font-size:11px;">
                  <span class="uuid-tip" style="cursor:pointer;" @click="copyUuid(p.rawuuid)">{{ p.rawuuid.slice(0, 8) }}…</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  </template>

  <!-- SMART health dialog (declarative modal, content loaded on demand) -->
  <div v-if="smartDialog" class="modal-overlay" @click.self="closeSmart">
    <div class="modal modal-wide" style="max-width:780px;">
      <div class="flex" style="justify-content:space-between;align-items:center;margin-bottom:14px;">
        <h3 style="margin:0;font-size:16px;">{{ smartDialog }} · SMART</h3>
        <div class="btn-group">
          <button v-if="smart[smartDialog] && smart[smartDialog].state === 'ok' && !smart[smartDialog].data.note && !smart[smartDialog].data.smartctl_missing" class="btn btn-sm" @click="loadSmart(smartDialog)">{{ t('common.refresh') }}</button>
          <button class="btn btn-sm" @click="closeSmart">{{ t('common.close') }}</button>
        </div>
      </div>

      <div v-if="!smart[smartDialog] || smart[smartDialog].state === 'loading'" class="text-dim" style="font-size:13px;"><span class="spinner"></span> {{ t('common.loading') }}</div>

      <div v-else-if="smart[smartDialog].state === 'error'" class="text-dim" style="font-size:13px;">{{ t('disks.smartLoadFailed', { msg: smart[smartDialog].error }) }}</div>

      <div v-else-if="smart[smartDialog].data.smartctl_missing" class="text-dim" style="font-size:13px;">
        <p style="margin:0 0 12px;">{{ t('disks.smartNeedInstall') }}</p>
        <div v-if="installTaskId">
          <TaskConsole :task-id="installTaskId" @done="onInstallDone" />
        </div>
        <button v-else class="btn" :disabled="installing" @click="installSmartmontools">
          <i class="fa-solid fa-download"></i> {{ t('pkg.installBtn') }} smartmontools
        </button>
      </div>
      <div v-else-if="smart[smartDialog].data.note" class="text-dim" style="font-size:13px;">{{ t('disks.smartUnsupported') }}<span style="font-size:12px;"> · {{ smart[smartDialog].data.note }}</span></div>

      <div v-else style="max-height:70vh;overflow:auto;">
        <div class="stat-grid">
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('common.status') }}</div>
            <span v-if="smart[smartDialog].data.healthy === true" class="badge badge-success">{{ t('disks.healthPassed') }}</span>
            <span v-else-if="smart[smartDialog].data.healthy === false" class="badge badge-danger">{{ t('disks.healthFailed') }}</span>
            <span v-else class="badge badge-warn">{{ t('disks.healthUnknown') }}</span>
          </div>
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.powerOnHours') }}</div>
            <div class="mono" style="font-size:13px;">{{ smart[smartDialog].data.power_on_hours != null ? smart[smartDialog].data.power_on_hours.toLocaleString() + ' h' : '—' }}</div>
          </div>
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.temperature') }}</div>
            <div class="mono" style="font-size:13px;" :style="smart[smartDialog].data.temperature >= 60 ? 'color:var(--danger)' : (smart[smartDialog].data.temperature >= 50 ? 'color:var(--warn)' : '')">{{ smart[smartDialog].data.temperature != null ? smart[smartDialog].data.temperature + ' °C' : '—' }}</div>
          </div>
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.powerCycles') }}</div>
            <div class="mono" style="font-size:13px;">{{ smart[smartDialog].data.power_cycle_count != null ? smart[smartDialog].data.power_cycle_count.toLocaleString() : '—' }}</div>
          </div>
        </div>

        <!-- NVMe wear & health -->
        <div v-if="smart[smartDialog].data.nvme" class="stat-grid" style="margin-top:12px;">
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.nvmeWear') }}</div>
            <div class="mono" style="font-size:13px;">{{ smart[smartDialog].data.nvme.percentage_used != null ? smart[smartDialog].data.nvme.percentage_used + ' %' : '—' }}</div>
          </div>
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.availableSpare') }}</div>
            <div class="mono" style="font-size:13px;">{{ smart[smartDialog].data.nvme.available_spare != null ? smart[smartDialog].data.nvme.available_spare + ' %' : '—' }}</div>
          </div>
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.mediaErrors') }}</div>
            <div class="mono" style="font-size:13px;">{{ smart[smartDialog].data.nvme.media_errors != null ? smart[smartDialog].data.nvme.media_errors.toLocaleString() : '—' }}</div>
          </div>
          <div>
            <div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.unsafeShutdowns') }}</div>
            <div class="mono" style="font-size:13px;">{{ smart[smartDialog].data.nvme.unsafe_shutdowns != null ? smart[smartDialog].data.nvme.unsafe_shutdowns.toLocaleString() : '—' }}</div>
          </div>
        </div>

        <!-- ATA attribute table -->
        <div v-if="smart[smartDialog].data.attributes.length" style="margin-top:14px;">
          <h2 style="font-size:14px;margin:0 0 8px;">{{ t('disks.attrTable') }}</h2>
          <table>
            <thead><tr>
              <th>ID</th><th>{{ t('common.name') }}</th><th>{{ t('common.value') }}</th><th>Thresh</th><th>Raw</th>
            </tr></thead>
            <tbody>
              <tr v-for="a in smart[smartDialog].data.attributes" :key="a.id">
                <td class="mono text-dim">{{ a.id }}</td>
                <td>{{ a.name }}</td>
                <td class="mono">{{ a.value != null ? a.value : '—' }}<span v-if="a.failing" class="badge badge-danger" style="margin-left:4px;">{{ t('disks.failing') }}</span></td>
                <td class="mono text-dim">{{ a.thresh != null ? a.thresh : '—' }}</td>
                <td class="mono">{{ a.raw_string || (a.raw != null ? a.raw : '—') }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  </div>
</template>
