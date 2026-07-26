<script setup>
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import TaskConsole from '../components/ui/TaskConsole.vue';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const initializing = ref(false);
const taskDone = ref(false);
const taskSuccess = ref(false);
const activeTaskId = ref('');

async function doInit() {
  initializing.value = true;
  taskDone.value = false;
  taskSuccess.value = false;
  activeTaskId.value = '';

  let taskId;
  try {
    const res = await api.post('/api/smb/init');
    taskId = res.task_id;
  } catch (e) {
    initializing.value = false;
    await alert(t('smb.initFailed'), e.message || t('smb.initFailed'));
    return;
  }

  activeTaskId.value = taskId;
}

async function onTaskDone({ success, output }) {
  taskDone.value = true;
  taskSuccess.value = success;
  initializing.value = false;
  if (success) {
    toast.toast(t('smb.initSuccess'));
    setTimeout(() => router.push('/shares/smb'), 2000);
  } else {
    await alert(t('smb.initFailed'), output.split('\n').filter(l => l).slice(-5).join('\n'));
  }
}
</script>

<template>
  <div class="page-header">
    <h1>{{ t('smb.initTitle') }}</h1>
    <p>{{ t('smb.initDesc') }}</p>
  </div>

  <div class="card" v-if="!activeTaskId && !initializing">
    <p class="text-dim">{{ t('smb.initWhatHappens') }}</p>
    <ol style="margin:8px 0;padding-left:20px;line-height:1.8;">
      <li>{{ t('smb.initStep1') }}</li>
      <li>{{ t('smb.initStep2') }}</li>
      <li>{{ t('smb.initStep3') }}</li>
    </ol>
  </div>

  <div class="card">
    <div v-if="activeTaskId || initializing" style="margin-bottom:16px;">
      <div class="flex" style="align-items:center;gap:8px;margin-bottom:8px;">
        <span v-if="!taskDone" class="spinner"></span>
        <strong>{{ t('smb.initConsole') }}</strong>
        <span v-if="taskDone && taskSuccess" class="badge badge-success">{{ t('common.done') }}</span>
        <span v-else-if="taskDone && !taskSuccess" class="badge badge-error">{{ t('common.failed') }}</span>
      </div>
      <TaskConsole :task-id="activeTaskId" @done="onTaskDone" />
    </div>

    <div class="btn-group">
      <button @click="doInit" :disabled="initializing">
        <i class="fa-solid fa-rocket"></i> {{ t('smb.initStart') }}
      </button>
      <button class="btn-secondary" @click="router.push('/shares/smb')" :disabled="initializing">
        {{ t('common.back') }}
      </button>
    </div>
  </div>
</template>
