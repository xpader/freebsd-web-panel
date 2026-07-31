<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import TaskConsole from '../components/ui/TaskConsole.vue';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const initType = ref('zfs');
const zfsDataset = ref('');
const dirPath = ref('');
const initializing = ref(false);
const taskDone = ref(false);
const taskSuccess = ref(false);
const error = ref('');
const activeTaskId = ref('');
const virtSupported = ref(true);

const spec = computed(() => {
  if (initType.value === 'zfs') {
    return zfsDataset.value.trim() ? `zfs:${zfsDataset.value.trim()}` : '';
  }
  return dirPath.value.trim();
});

const canSubmit = computed(() => {
  if (initializing.value) return false;
  if (!virtSupported.value) return false;
  if (initType.value === 'zfs') return zfsDataset.value.trim().length > 0;
  return dirPath.value.trim().startsWith('/');
});

function scrollToBottom() {
  nextTick(() => {
    if (consoleRef.value) consoleRef.value.scrollTop = consoleRef.value.scrollHeight;
  });
}

async function doInit() {
  if (!canSubmit.value) return;
  initializing.value = true;
  error.value = '';
  taskDone.value = false;
  taskSuccess.value = false;
  activeTaskId.value = '';

  let taskId;
  try {
    const res = await api.post('/api/bhyve/init', { spec: spec.value });
    taskId = res.task_id;
  } catch (e) {
    initializing.value = false;
    error.value = e.message || '';
    await alert(t('bhyve.initFailed'), error.value);
    return;
  }

  activeTaskId.value = taskId;
}

async function onTaskDone({ success, output }) {
  taskDone.value = true;
  taskSuccess.value = success;
  initializing.value = false;
  if (success) {
    toast.toast(t('bhyve.initSuccess'));
    setTimeout(() => router.push('/bhyve/vms'), 2000);
  } else {
    await alert(t('bhyve.initFailed'), output.split('\n').filter(l => l).slice(-5).join('\n'));
  }
}

onMounted(async () => {
  try {
    const s = await api.get('/api/bhyve/status');
    virtSupported.value = s.virt_supported;
  } catch {}
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.initTitle') }}</h1>
    <p>{{ t('bhyve.initDesc') }}</p>
  </div>

  <div v-if="!virtSupported" class="card" style="border:1px solid var(--danger);">
    <div class="flex" style="align-items:flex-start;gap:12px;">
      <i class="fa-solid fa-circle-exclamation" style="font-size:24px;color:var(--danger);margin-top:2px;"></i>
      <div>
        <h3 style="margin:0 0 8px;color:var(--danger);">{{ t('bhyve.initVirtUnsupported') }}</h3>
        <p style="font-size:13px;line-height:1.6;">{{ t('bhyve.initVirtUnsupportedDesc') }}</p>
      </div>
    </div>
    <div class="btn-group" style="margin-top:16px;">
      <button class="btn-secondary" @click="router.push('/bhyve/vms')">{{ t('common.back') }}</button>
    </div>
  </div>

  <template v-else>
  <div class="card">
    <h3>{{ t('bhyve.initStorageType') }}</h3>
    <div class="radio-pill-group" style="margin-bottom:16px;">
      <label class="radio-pill" :class="{ active: initType === 'zfs' }">
        <input type="radio" value="zfs" v-model="initType" :disabled="initializing" />
        <span>ZFS {{ t('bhyve.initDataset') }}</span>
      </label>
      <label class="radio-pill" :class="{ active: initType === 'directory' }">
        <input type="radio" value="directory" v-model="initType" :disabled="initializing" />
        <span>{{ t('bhyve.initDirectory') }}</span>
      </label>
    </div>

    <div v-if="initType === 'zfs'" class="field">
      <label>{{ t('bhyve.initZfsDataset') }}</label>
      <input
        v-model="zfsDataset"
        :placeholder="t('bhyve.initZfsPlaceholder')"
        :disabled="initializing"
      />
      <p class="text-dim" style="font-size:12px;margin-top:4px;">{{ t('bhyve.initZfsHint') }}</p>
    </div>

    <div v-else class="field">
      <label>{{ t('bhyve.initDirPath') }}</label>
      <input
        v-model="dirPath"
        :placeholder="t('bhyve.initDirPlaceholder')"
        :disabled="initializing"
      />
      <p class="text-dim" style="font-size:12px;margin-top:4px;">{{ t('bhyve.initDirHint') }}</p>
    </div>

    <div v-if="activeTaskId || initializing" style="margin:16px 0;">
      <div class="flex" style="align-items:center;gap:8px;margin-bottom:8px;">
        <span v-if="!taskDone" class="spinner"></span>
        <strong>{{ t('bhyve.initConsole') }}</strong>
        <span v-if="taskDone && taskSuccess" class="badge badge-success">{{ t('common.done') }}</span>
        <span v-else-if="taskDone && !taskSuccess" class="badge badge-error">{{ t('common.failed') }}</span>
      </div>
      <TaskConsole :task-id="activeTaskId" @done="onTaskDone" />
    </div>

    <div v-if="error && !activeTaskId" class="alert-error" style="margin:16px 0;">
      <strong>{{ t('common.operationFailed') }}</strong>: {{ error }}
    </div>

    <div class="btn-group">
      <button @click="doInit" :disabled="!canSubmit">
        <i class="fa-solid fa-rocket"></i> {{ t('bhyve.initStart') }}
      </button>
      <button class="btn-secondary" @click="router.push('/bhyve/vms')" :disabled="initializing">
        {{ t('common.cancel') }}
      </button>
    </div>
  </div>

  <div class="card" v-if="!activeTaskId && !initializing">
    <h3>{{ t('bhyve.initWhatHappens') }}</h3>
    <ol style="margin:8px 0;padding-left:20px;line-height:1.8;">
      <li>{{ t('bhyve.initStep1') }}</li>
      <li>{{ t('bhyve.initStep2') }}</li>
      <li>{{ t('bhyve.initStep3') }}</li>
      <li>{{ t('bhyve.initStep4') }}</li>
      <li>{{ t('bhyve.initStep5') }}</li>
    </ol>
  </div>
  </template>
</template>
