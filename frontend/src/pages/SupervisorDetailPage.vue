<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtUptime, fmtBytes, fmtTime } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import StatusBar from '../components/shared/StatusBar.vue';
import BackButton from '../components/ui/BackButton.vue';
import { openSupervisorForm } from '../lib/supervisorForm.js';

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();
const id = Number(route.params.id);

const p = ref(null);
const error = ref('');
const busy = ref(false);
const logContent = ref('');
const logSize = ref(0);

let pollTimer = null;

const statusItems = computed(() => {
  if (!p.value) return [];
  return [
    { title: t('common.status'), value: stateLabel(p.value.state), type: 'badge', status: stateStatus(p.value.state) },
    { title: 'PID', value: p.value.pid ?? '—' },
    { title: 'Supervisor PID', value: p.value.sup_pid ?? '—' },
    { title: t('supv.uptime'), value: p.value.uptime != null ? fmtUptime(p.value.uptime) : '—' },
  ];
});

function stateStatus(s) {
  if (s === 'running') return 'ok';
  if (s === 'starting') return 'warning';
  if (s === 'orphaned') return 'error';
  return 'inactive';
}

function badgeClass(s) {
  return ['badge', stateStatus(s) === 'ok' ? 'badge-success'
    : stateStatus(s) === 'warning' ? 'badge-warn'
    : stateStatus(s) === 'error' ? 'badge-error' : 'badge-dim'];
}

function stateLabel(s) {
  if (s === 'running') return t('common.running');
  if (s === 'starting') return t('supv.starting');
  if (s === 'orphaned') return t('supv.orphaned');
  return t('common.stopped');
}

function restartSummary() {
  const d = p.value;
  if (!d.restart) return t('common.no');
  return `${d.restart_delay}s · ${d.restart_max != null ? `×${d.restart_max}` : '∞'}`;
}

async function load() {
  try {
    p.value = await api.get(`/api/supervisor/${encodeURIComponent(id)}`);
    error.value = '';
  } catch (e) {
    error.value = e.message || '';
  }
}

async function fetchLog() {
  if (!p.value || !p.value.logging) return;
  try {
    const res = await api.get(`/api/supervisor/${encodeURIComponent(id)}/log?lines=500`);
    logContent.value = res.content;
    logSize.value = res.size;
  } catch {
    // Log read is a background refresh; surface via size/content only.
  }
}

async function refreshAll() {
  await load();
  await fetchLog();
}

async function control(action) {
  if (busy.value) return;
  busy.value = true;
  try {
    await api.post(`/api/supervisor/${encodeURIComponent(id)}/${action}`);
    const key = action === 'restart' ? 'restarted' : action === 'stop' ? 'stopped' : 'started';
    toast.toast(t(`supv.${key}`, { name: p.value.name }));
    await refreshAll();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

function showForm() {
  const d = p.value;
  if (d.state !== 'stopped') {
    toast.toast(t('supv.stopFirst'));
    return;
  }
  openSupervisorForm(formModal, t, toast, d, refreshAll);
}

async function removeProc() {
  const d = p.value;
  if (!await confirm(t('common.delete'), t('supv.deleteConfirm', { name: d.name }))) return;
  try {
    await api.del(`/api/supervisor/${encodeURIComponent(id)}`);
    toast.toast(t('supv.deleted', { name: d.name }));
    router.push('/supervisor');
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.operationFailed'));
  }
}

async function clearLog() {
  try {
    await api.del(`/api/supervisor/${encodeURIComponent(id)}/log`);
    await fetchLog();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(() => {
  refreshAll();
  pollTimer = setInterval(refreshAll, 5000);
});

onUnmounted(() => {
  clearInterval(pollTimer);
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/supervisor" />
      <h1>{{ p?.name || '…' }}</h1>
      <span v-if="p" :class="badgeClass(p.state)">
        {{ stateLabel(p.state) }}
      </span>
    </div>
    <div v-if="p" class="flex btn-group" style="margin-left:auto;">
      <button v-if="p.state === 'stopped'" class="btn-sm" :disabled="busy" @click="control('start')">
        <i class="fa-solid fa-play"></i> {{ t('common.start') }}
      </button>
      <button v-else class="btn-secondary btn-sm" :disabled="busy" @click="control('stop')">
        <i class="fa-solid fa-stop"></i> {{ t('common.stop') }}
      </button>
      <button class="btn-secondary btn-sm" :disabled="busy || p.state === 'stopped'" @click="control('restart')">
        <i class="fa-solid fa-rotate-right"></i> {{ t('common.restart') }}
      </button>
      <button class="btn-secondary btn-sm" :disabled="p.state !== 'stopped'" :title="p.state !== 'stopped' ? t('supv.stopFirst') : ''" @click="showForm">
        <i class="fa-solid fa-pen-to-square"></i> {{ t('common.edit') }}
      </button>
      <button class="btn-danger btn-sm" :disabled="p.state !== 'stopped'" :title="p.state !== 'stopped' ? t('supv.stopFirst') : ''" @click="removeProc">
        <i class="fa-solid fa-trash"></i> {{ t('common.delete') }}
      </button>
    </div>
  </div>

  <div v-if="error" class="card text-dim" style="padding:24px;text-align:center;">
    {{ t('common.loadFailed', { msg: error }) }}
  </div>
  <div v-else-if="!p" class="card text-dim" style="padding:24px;text-align:center;">
    <span class="spinner"></span> {{ t('common.loading') }}
  </div>

  <template v-else>
    <StatusBar :items="statusItems">
      <template #actions>
        <button class="btn-secondary btn-sm" @click="refreshAll">
          <i class="fa-solid fa-rotate-right"></i> {{ t('common.refresh') }}
        </button>
      </template>
    </StatusBar>

    <!-- Basic info -->
    <div class="card">
      <h3>{{ t('common.basicInfo') }}</h3>
      <table class="kv-table four-col">
        <tbody>
        <tr>
          <td>{{ t('supv.command') }}</td>
          <td class="mono" colspan="3">{{ p.path }}<template v-if="p.args?.length"> {{ p.args.join(' ') }}</template></td>
        </tr>
        <tr>
          <td>{{ t('supv.workdir') }}</td>
          <td class="mono">{{ p.workdir || '—' }}</td>
          <td>{{ t('supv.runUser') }}</td>
          <td>{{ p.user || 'root' }}</td>
        </tr>
        <tr v-if="p.env?.length">
          <td>{{ t('supv.envVars') }}</td>
          <td class="mono" colspan="3">{{ p.env.join(' · ') }}</td>
        </tr>
        <tr>
          <td>{{ t('supv.restartPolicy') }}</td>
          <td>{{ restartSummary() }}</td>
          <td>{{ t('supv.autostart') }}</td>
          <td>{{ p.autostart ? t('common.yes') : t('common.no') }}</td>
        </tr>
        <tr v-if="p.log_file">
          <td>{{ t('supv.logFile') }}</td>
          <td class="mono" colspan="3">{{ p.log_file }}</td>
        </tr>
        <tr>
          <td>{{ t('supv.logging') }}</td>
          <td>{{ p.logging ? t('common.enabled') : t('common.disabled') }}</td>
          <td>{{ t('supv.logRotate') }}</td>
          <td>{{ p.logging ? (p.log_rotate ? t('common.enabled') : t('common.disabled')) : '—' }}</td>
        </tr>
        <tr>
          <td>{{ t('common.createdAt') }}</td>
          <td>{{ fmtTime(p.created_at) }}</td>
          <td>{{ t('common.updatedAt') }}</td>
          <td>{{ fmtTime(p.updated_at) }}</td>
        </tr>
        </tbody>
      </table>
    </div>

    <!-- Log -->
    <div class="card">
      <div class="flex" style="align-items:center;gap:8px;margin-bottom:8px;">
        <h3 style="margin:0;">{{ t('supv.log') }}</h3>
        <span v-if="p.log_file" class="text-dim mono" style="font-size:12px;">{{ p.log_file }}</span>
        <span v-if="p.logging" class="text-dim" style="font-size:12px;">{{ fmtBytes(logSize) }}</span>
        <div class="btn-group" style="margin-left:auto;">
          <button class="btn-secondary btn-sm" :disabled="!p.logging" @click="fetchLog">
            <i class="fa-solid fa-rotate-right"></i> {{ t('common.refresh') }}
          </button>
          <button class="btn-secondary btn-sm" :disabled="!p.logging" @click="clearLog">
            <i class="fa-solid fa-eraser"></i> {{ t('common.clear') }}
          </button>
        </div>
      </div>
      <div v-if="!p.logging" class="text-dim" style="padding:16px 0;text-align:center;">
        {{ t('supv.logDisabledTitle') }}
      </div>
      <pre v-else class="mono" style="max-height:420px;overflow:auto;margin:0;white-space:pre-wrap;">{{ logContent || t('supv.logEmpty') }}</pre>
    </div>
  </template>
</template>
