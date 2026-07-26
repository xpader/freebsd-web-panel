<script setup>
import { ref, nextTick, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const initializing = ref(false);
const taskOutput = ref('');
const taskDone = ref(false);
const taskSuccess = ref(false);
const consoleRef = ref(null);
let taskEs = null;

function scrollToBottom() {
  nextTick(() => {
    if (consoleRef.value) {
      consoleRef.value.scrollTop = consoleRef.value.scrollHeight;
    }
  });
}

async function doInit() {
  initializing.value = true;
  taskOutput.value = '';
  taskDone.value = false;
  taskSuccess.value = false;

  let taskId;
  try {
    const res = await api.post('/api/smb/init');
    taskId = res.task_id;
  } catch (e) {
    initializing.value = false;
    await alert(t('smb.initFailed'), e.message || t('smb.initFailed'));
    return;
  }

  const token = sessionStorage.getItem('fwp_token');
  const url = `/api/tasks/${encodeURIComponent(taskId)}/stream?token=${encodeURIComponent(token)}`;
  const es = new EventSource(url);
  taskEs = es;

  const finish = async (success) => {
    es.close();
    taskDone.value = true;
    taskSuccess.value = success;
    initializing.value = false;
    if (success) {
      taskOutput.value += `\n[${t('common.done')}]\n`;
      toast.toast(t('smb.initSuccess'));
      setTimeout(() => router.push('/shares/smb'), 2000);
    } else {
      await alert(t('smb.initFailed'), taskOutput.value.split('\n').filter(l => l).slice(-5).join('\n'));
    }
  };

  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data.lines && data.lines.length) {
        taskOutput.value += data.lines.join('\n') + '\n';
        scrollToBottom();
      }
      if (data.status && data.status !== 'running') {
        finish(data.status === 'done');
      }
    } catch {}
  };
  es.addEventListener('done', () => { es.close(); taskDone.value = true; initializing.value = false; });
  es.onerror = () => {
    es.close();
    api.get(`/api/tasks/${encodeURIComponent(taskId)}`).then((task) => {
      if (task.status !== 'running') {
        finish(task.status === 'done');
      } else {
        taskDone.value = true;
        initializing.value = false;
      }
    }).catch(() => { taskDone.value = true; initializing.value = false; });
  };
}

onUnmounted(() => {
  if (taskEs) taskEs.close();
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('smb.initTitle') }}</h1>
    <p>{{ t('smb.initDesc') }}</p>
  </div>

  <div class="card" v-if="!taskOutput && !initializing">
    <p class="text-dim">{{ t('smb.initWhatHappens') }}</p>
    <ol style="margin:8px 0;padding-left:20px;line-height:1.8;">
      <li>{{ t('smb.initStep1') }}</li>
      <li>{{ t('smb.initStep2') }}</li>
      <li>{{ t('smb.initStep3') }}</li>
    </ol>
  </div>

  <div class="card">
    <div v-if="taskOutput || initializing" style="margin-bottom:16px;">
      <div class="flex" style="align-items:center;gap:8px;margin-bottom:8px;">
        <span v-if="!taskDone" class="spinner"></span>
        <strong>{{ t('smb.initConsole') }}</strong>
        <span v-if="taskDone && taskSuccess" class="badge badge-success">{{ t('common.done') }}</span>
        <span v-else-if="taskDone && !taskSuccess" class="badge badge-error">{{ t('common.failed') }}</span>
      </div>
      <div
        ref="consoleRef"
        style="max-height:400px; overflow-y:auto; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius); padding:12px; font-family:monospace; font-size:12px; white-space:pre-wrap; word-break:break-all;"
      >{{ taskOutput }}</div>
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
