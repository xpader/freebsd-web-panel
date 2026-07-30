<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import TaskConsole from '../components/ui/TaskConsole.vue';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const tasks = ref([]);
const loading = ref(true);
const error = ref('');
const rsyncStatus = ref(null);

// Manual-run state
const runTarget = ref(null);
const runDry = ref(false);
const runTaskId = ref('');
const runDone = ref(false);
const runSuccess = ref(false);
const runOutput = ref('');

const formModal = useFormModal();

const needsInit = computed(() => {
  if (!rsyncStatus.value) return false;
  return !rsyncStatus.value.installed;
});

function statusBadge(s) {
  if (!s) return 'badge-dim';
  if (s === 'success') return 'badge-success';
  if (s === 'failed') return 'badge-error';
  return 'badge-dim';
}

async function loadStatus() {
  try {
    rsyncStatus.value = await api.get('/api/rsync/status');
  } catch { rsyncStatus.value = null; }
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    tasks.value = await api.get('/api/rsync/tasks');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function showForm(existing = null) {
  const isEdit = !!existing;
  // Source/dest use `picker: 'dir'` + `portKey: 'port'`. DialogHost auto-detects
  // local vs remote from the field value: an SSH-style spec opens RemoteFilePicker
  // (port taken from the `port` field), otherwise the local FilePicker.
  const result = await formModal(
    isEdit ? t('rsync.edit') : t('rsync.create'),
    [
      { key: 'description', label: t('common.description'), value: existing?.description || '', required: true },
      { key: 'source', label: t('rsync.source'), value: existing?.source || '', placeholder: '/zroot/data 或 user@host:/path', required: true, picker: 'dir', portKey: 'port' },
      { key: 'dest', label: t('rsync.dest'), value: existing?.dest || '', placeholder: 'user@host:/backup 或 /mnt/backup', required: true, picker: 'dir', portKey: 'port' },
      {
        key: '_flags', label: t('common.options'), type: 'checkbox-group',
        options: [
          { key: 'archive', label: t('rsync.archive'), value: existing?.archive ?? true },
          { key: 'compress', label: t('rsync.compress'), value: existing?.compress ?? false },
          { key: 'delete', label: t('rsync.deleteExtras'), value: existing?.delete ?? false },
          { key: 'verbose', label: t('rsync.verbose'), value: existing?.verbose ?? true },
        ],
      },
      { key: 'port', label: t('rsync.port'), value: existing?.port ?? '', placeholder: '22', half: true },
      { key: 'extra_args', label: t('rsync.extraArgs'), value: existing?.extra_args || '', placeholder: '--partial --bwlimit=1000', help: t('rsync.extraArgsHint') },
      { key: 'run_user', label: t('rsync.runUser'), value: existing?.run_user || '', placeholder: 'root', help: t('rsync.runUserHint'), half: true },
      { key: 'cron', type: 'cron', value: { enabled: !!existing?.cron_enabled, expr: existing?.cron_expr || '' } },
    ],
    {
      submitLabel: isEdit ? t('common.save') : t('common.create'),
      // submitHandler: validation/API errors display inline in the dialog
      // (DialogHost catches the throw, keeps the modal open) instead of
      // closing it and showing a separate alert.
      submitHandler: async (r) => {
        const portVal = parseInt(r.port, 10);
        const body = {
          description: r.description.trim(),
          source: r.source.trim(),
          dest: r.dest.trim(),
          archive: !!r.archive,
          compress: !!r.compress,
          delete: !!r.delete,
          verbose: !!r.verbose,
          port: Number.isFinite(portVal) ? portVal : null,
          extra_args: r.extra_args || '',
          run_user: r.run_user?.trim() || '',
          cron_expr: r.cron?.expr?.trim() || '',
          cron_enabled: !!r.cron?.enabled,
        };
        if (isEdit) {
          await api.put(`/api/rsync/tasks/${encodeURIComponent(existing.id)}`, body);
          toast.toast(t('rsync.updated', { desc: existing.description }));
        } else {
          await api.post('/api/rsync/tasks', body);
          toast.toast(t('rsync.created', { desc: body.description }));
        }
        await load();
      },
    },
  );
}

async function deleteTask(task) {
  if (!await confirm(t('common.delete'), t('rsync.deleteConfirm', { desc: task.description }))) return;
  try {
    await api.del(`/api/rsync/tasks/${encodeURIComponent(task.id)}`);
    toast.toast(t('rsync.deleted', { desc: task.description }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function startRun(task, dry = false) {
  if (runTarget.value) return;
  runTarget.value = task;
  runDry.value = dry;
  runDone.value = false;
  runSuccess.value = false;
  runOutput.value = '';
  runTaskId.value = '';
  try {
    const res = await api.post(`/api/rsync/tasks/${encodeURIComponent(task.id)}/run`, { dry_run: dry });
    runTaskId.value = res.task_id;
  } catch (e) {
    runTarget.value = null;
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function onRunDone({ success, output }) {
  runDone.value = true;
  runSuccess.value = success;
  runOutput.value = output;
  await load();
  if (success) toast.toast(t('rsync.runSuccess'));
  else await alert(t('rsync.runFailed'), output.split('\n').filter(l => l).slice(-8).join('\n'));
}

function closeRun() {
  runTarget.value = null;
  runTaskId.value = '';
  runDone.value = false;
  runSuccess.value = false;
  runOutput.value = '';
}

onMounted(async () => {
  await loadStatus();
  if (!needsInit.value) load();
  else loading.value = false;
});

onUnmounted(() => {
  runTarget.value = null;
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('rsync.title') }}</h1>
    <p>{{ t('rsync.subtitle') }}</p>
    <div style="margin-left:auto;" class="btn-group">
      <button @click="showForm()"><i class="fa-solid fa-plus"></i> {{ t('rsync.create') }}</button>
      <button class="btn-secondary" @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <!-- Not installed banner -->
  <div v-if="needsInit" class="card" style="border:1px solid var(--warning);padding:24px;text-align:center;">
    <i class="fa-solid fa-triangle-exclamation" style="font-size:32px;color:var(--warning);"></i>
    <h3 style="margin:12px 0 8px;">{{ t('rsync.initRequired') }}</h3>
    <p class="text-dim" style="margin-bottom:8px;">{{ t('rsync.initRequiredDesc') }}</p>
    <div class="btn-group" style="justify-content:center;">
      <button @click="router.push('/rsync/init')">
        <i class="fa-solid fa-rocket"></i> {{ t('rsync.initGo') }}
      </button>
    </div>
  </div>

  <template v-else>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('common.description') }}</th>
          <th>{{ t('rsync.source') }}</th>
          <th>{{ t('rsync.dest') }}</th>
          <th>{{ t('common.options') }}</th>
          <th>{{ t('rsync.lastStatus') }}</th>
          <th>{{ t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
          <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
          <tr v-else-if="!tasks.length"><td colspan="6" class="empty">{{ t('rsync.noTasks') }}</td></tr>
          <tr v-for="task in tasks" :key="task.id">
            <td class="mono">
              <strong>{{ task.description }}</strong>
              <span v-if="task.cron_enabled && task.cron_expr" class="badge badge-info" style="margin-left:6px;">{{ task.cron_expr }}</span>
              <span v-else-if="task.cron_expr" class="badge badge-dim" style="margin-left:6px;">{{ task.cron_expr }} ({{ t('common.disabled') }})</span>
            </td>
            <td class="mono">{{ task.source }}</td>
            <td class="mono">{{ task.dest }}</td>
            <td>
              <span class="badge badge-dim" v-if="task.archive" style="margin-right:4px;">-a</span>
              <span class="badge badge-dim" v-if="task.compress" style="margin-right:4px;">-z</span>
              <span class="badge badge-dim" v-if="task.delete" style="margin-right:4px;">--delete</span>
              <span class="badge badge-dim" v-if="task.verbose" style="margin-right:4px;">-v</span>
            </td>
            <td><span :class="['badge', statusBadge(task.last_status)]">{{ task.last_status ? (task.last_status === 'success' ? t('rsync.statusSuccess') : t('rsync.statusFailed')) : '—' }}</span></td>
            <td>
              <div class="btn-group">
                <button class="btn-sm" :disabled="!!runTarget" @click="startRun(task, false)" :title="t('rsync.run')"><i class="fa-solid fa-play"></i></button>
                <button class="btn-secondary btn-sm" :disabled="!!runTarget" @click="startRun(task, true)" :title="t('rsync.runDry')"><i class="fa-solid fa-flask"></i></button>
                <button class="btn-secondary btn-sm" @click="showForm(task)"><i class="fa-solid fa-pen"></i></button>
                <button class="btn-danger btn-sm" @click="deleteTask(task)"><i class="fa-solid fa-trash"></i></button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Run console -->
    <div v-if="runTarget" class="card">
      <div class="flex" style="align-items:center;gap:8px;margin-bottom:8px;">
        <span v-if="!runDone" class="spinner"></span>
        <strong>{{ t('rsync.runLog') }} — {{ runTarget.description }}</strong>
        <span v-if="runDry" class="badge badge-warn">{{ t('rsync.dryRunBadge') }}</span>
        <span v-if="runDone && runSuccess" class="badge badge-success">{{ t('common.done') }}</span>
        <span v-else-if="runDone && !runSuccess" class="badge badge-error">{{ t('common.failed') }}</span>
        <div style="margin-left:auto;" class="btn-group">
          <button v-if="runDone" class="btn-secondary btn-sm" @click="closeRun">{{ t('common.close') }}</button>
        </div>
      </div>
      <TaskConsole :task-id="runTaskId" @done="onRunDone" />
    </div>
  </template>

</template>
