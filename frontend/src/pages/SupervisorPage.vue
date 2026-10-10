<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtUptime } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import { openSupervisorForm } from '../lib/supervisorForm.js';

const router = useRouter();
const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const procs = ref([]);
const loading = ref(true);
const error = ref('');
const filter = ref('');
const busy = ref(false);

let pollTimer = null;

const filtered = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return procs.value;
  return procs.value.filter((p) =>
    p.name.toLowerCase().includes(q) ||
    p.path.toLowerCase().includes(q) ||
    (p.args || []).join(' ').toLowerCase().includes(q));
});

function stateBadge(s) {
  switch (s) {
    case 'running': return 'badge badge-success';
    case 'starting': return 'badge badge-warn';
    case 'orphaned': return 'badge badge-error';
    default: return 'badge badge-dim';
  }
}

function stateLabel(s) {
  if (s === 'running') return t('common.running');
  if (s === 'starting') return t('supv.starting');
  if (s === 'orphaned') return t('supv.orphaned');
  return t('common.stopped');
}

function restartSummary(p) {
  if (!p.restart) return t('common.no');
  return `${p.restart_delay}s · ${p.restart_max != null ? `×${p.restart_max}` : '∞'}`;
}

async function load() {
  try {
    procs.value = await api.get('/api/supervisor');
    error.value = '';
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

function showForm(existing = null) {
  openSupervisorForm(formModal, t, toast, existing, load);
}

async function control(p, action) {
  if (busy.value) return;
  busy.value = true;
  try {
    await api.post(`/api/supervisor/${encodeURIComponent(p.id)}/${action}`);
    const key = action === 'restart' ? 'restarted' : action === 'stop' ? 'stopped' : 'started';
    toast.toast(t(`supv.${key}`, { name: p.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

async function removeProc(p) {
  if (!await confirm(t('common.delete'), t('supv.deleteConfirm', { name: p.name }))) return;
  try {
    await api.del(`/api/supervisor/${encodeURIComponent(p.id)}`);
    toast.toast(t('supv.deleted', { name: p.name }));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(() => {
  load();
  pollTimer = setInterval(load, 5000);
});

onUnmounted(() => {
  clearInterval(pollTimer);
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('nav.processGuard') }}</h1>
    <p>{{ t('supv.subtitle') }}</p>
  </div>

  <div class="toolbar">
    <SearchInput v-model="filter" :placeholder="t('common.search')" />
    <span class="text-dim">{{ t('supv.count', { n: filtered.length }) }}</span>
    <div class="btn-group" style="margin-left:auto;">
      <button class="btn-secondary" @click="load" :disabled="loading">
        <i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}
      </button>
      <button @click="showForm()"><i class="fa-solid fa-plus"></i> {{ t('common.create') }}</button>
    </div>
  </div>

  <div v-if="loading" class="card text-dim" style="padding:24px;text-align:center;">
    <span class="spinner"></span> {{ t('common.loading') }}
  </div>
  <div v-else-if="error" class="card" style="padding:24px;text-align:center;">
    {{ t('common.loadFailed', { msg: error }) }}
  </div>
  <template v-else>
    <div v-if="!filtered.length" class="card text-dim" style="padding:24px;text-align:center;">
      {{ t('supv.noProcs') }}
    </div>
    <div v-else class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('supv.command') }}</th>
          <th>{{ t('common.status') }}</th>
          <th>PID</th>
          <th>{{ t('supv.uptime') }}</th>
          <th>{{ t('supv.restartPolicy') }}</th>
          <th>{{ t('supv.autostart') }}</th>
          <th>{{ t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="p in filtered" :key="p.id" class="row-clickable" @click="router.push(`/supervisor/${p.id}`)">
            <td>
              <strong>{{ p.name }}</strong>
              <div class="text-dim" style="font-size:12px;">{{ p.user || 'root' }}</div>
            </td>
            <td><code class="mono" style="font-size:12px;">{{ p.path }}<template v-if="p.args && p.args.length"> {{ p.args.join(' ') }}</template></code></td>
            <td><span :class="stateBadge(p.state)">{{ stateLabel(p.state) }}</span></td>
            <td>{{ p.pid != null ? p.pid : '—' }}</td>
            <td>{{ p.uptime != null ? fmtUptime(p.uptime) : '—' }}</td>
            <td>{{ restartSummary(p) }}</td>
            <td><i v-if="p.autostart" class="fa-solid fa-check" style="color:var(--success);"></i><span v-else class="text-dim">—</span></td>
            <td>
              <div class="btn-group" @click.stop>
                <button class="btn-secondary btn-sm" :disabled="busy || p.state !== 'stopped'" @click="control(p, 'start')" :title="t('common.start')"><i class="fa-solid fa-play"></i></button>
                <button class="btn-secondary btn-sm" :disabled="busy || p.state === 'stopped'" @click="control(p, 'stop')" :title="t('common.stop')"><i class="fa-solid fa-stop"></i></button>
                <button class="btn-secondary btn-sm" :disabled="busy || p.state === 'stopped'" @click="control(p, 'restart')" :title="t('common.restart')"><i class="fa-solid fa-rotate-right"></i></button>
                <button class="btn-secondary btn-sm" :disabled="p.state !== 'stopped'" @click="showForm(p)" :title="p.state !== 'stopped' ? t('supv.stopFirst') : t('common.edit')"><i class="fa-solid fa-pen"></i></button>
                <button class="btn-danger btn-sm" :disabled="p.state !== 'stopped'" @click="removeProc(p)" :title="p.state !== 'stopped' ? t('supv.stopFirst') : t('common.delete')"><i class="fa-solid fa-trash"></i></button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
