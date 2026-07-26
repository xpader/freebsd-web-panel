<script setup>
import { ref, watch, onUnmounted, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';

const props = defineProps({
  taskId: { type: String, default: '' },
});
const emit = defineEmits(['done']);

const { t } = useI18n();
const output = ref('');
const consoleRef = ref(null);
let es = null;

function scroll() {
  nextTick(() => {
    if (consoleRef.value) consoleRef.value.scrollTop = consoleRef.value.scrollHeight;
  });
}

function start(taskId) {
  output.value = '';
  const token = sessionStorage.getItem('fwp_token');
  const url = `/api/tasks/${encodeURIComponent(taskId)}/stream?token=${encodeURIComponent(token)}`;
  es = new EventSource(url);

  const finish = (success) => {
    es?.close();
    es = null;
    if (success) {
      output.value += `\n[${t('common.done')}]\n`;
    }
    scroll();
    emit('done', { success, output: output.value });
  };

  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data.lines?.length) {
        output.value += data.lines.join('\n') + '\n';
        scroll();
      }
      if (data.status && data.status !== 'running') {
        finish(data.status === 'done');
      }
    } catch {}
  };
  es.addEventListener('done', () => { es?.close(); es = null; });
  es.onerror = () => {
    es?.close();
    es = null;
    api.get(`/api/tasks/${encodeURIComponent(taskId)}`).then((task) => {
      if (task.status !== 'running') {
        finish(task.status === 'done');
      }
    }).catch(() => finish(false));
  };
}

watch(() => props.taskId, (id, oldId) => {
  if (oldId && es) { es.close(); es = null; }
  if (id) start(id);
}, { immediate: true });

onUnmounted(() => { if (es) es.close(); });
</script>

<template>
  <div
    ref="consoleRef"
    style="max-height:400px; overflow-y:auto; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius); padding:12px; font-family:monospace; font-size:12px; white-space:pre-wrap; word-break:break-all;"
  >{{ output }}</div>
</template>
