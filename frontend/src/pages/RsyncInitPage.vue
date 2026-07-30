<script setup>
import { ref } from 'vue';
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
    const res = await api.post('/api/rsync/init');
    taskId = res.task_id;
  } catch (e) {
    initializing.value = false;
    await alert(t('rsync.initFailed'), e.message || t('rsync.initFailed'));
    return;
  }

  activeTaskId.value = taskId;
}

async function onTaskDone({ success, output }) {
  taskDone.value = true;
  taskSuccess.value = success;
  initializing.value = false;
  if (success) {
    toast.toast(t('rsync.initSuccess'));
    setTimeout(() => router.push('/rsync'), 2000);
  } else {
    await alert(t('rsync.initFailed'), output.split('\n').filter(l => l).slice(-5).join('\n'));
  }
}
</script>

<template>
  <div class="page-header">
    <h1>{{ t('rsync.initTitle') }}</h1>
    <p>{{ t('rsync.initDesc') }}</p>
  </div>

  <div class="card" v-if="!activeTaskId && !initializing">
    <p class="text-dim">{{ t('rsync.initWhatHappens') }}</p>
    <ol style="margin:8px 0;padding-left:20px;line-height:1.8;">
      <li>{{ t('rsync.initStep1') }}</li>
    </ol>
  </div>

  <div class="card">
    <div v-if="activeTaskId || initializing" style="margin-bottom:16px;">
      <div class="flex" style="align-items:center;gap:8px;margin-bottom:8px;">
        <span v-if="!taskDone" class="spinner"></span>
        <strong>{{ t('rsync.initConsole') }}</strong>
        <span v-if="taskDone && taskSuccess" class="badge badge-success">{{ t('common.done') }}</span>
        <span v-else-if="taskDone && !taskSuccess" class="badge badge-error">{{ t('common.failed') }}</span>
      </div>
      <TaskConsole :task-id="activeTaskId" @done="onTaskDone" />
    </div>

    <div class="btn-group">
      <button @click="doInit" :disabled="initializing">
        <i class="fa-solid fa-rocket"></i> {{ t('rsync.initStart') }}
      </button>
      <button class="btn-secondary" @click="router.push('/rsync')" :disabled="initializing">
        {{ t('common.cancel') }}
      </button>
    </div>
  </div>
</template>
