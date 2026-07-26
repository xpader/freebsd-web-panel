<script setup>
import { ref, computed, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';
import { useToast, useAlert } from '../../composables/useDialog.js';
import StatusBar from './StatusBar.vue';

const props = defineProps({
  status: { type: Object, required: true },
});

const emit = defineEmits(['refresh']);

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const pendingAction = ref('');

const serviceRunning = computed(() => props.status?.service_running ?? false);

const items = computed(() => {
  if (!props.status) return [];
  return [
    { title: t('common.status'), value: serviceRunning.value ? t('common.running') : t('common.stopped'), type: 'badge', status: serviceRunning.value ? 'ok' : 'inactive' },
    ...(props.status.version ? [{ title: t('smb.version'), value: `Samba ${props.status.version}` }] : []),
    { title: t('smb.autoStart'), value: props.status.enabled ? t('common.yes') : t('common.no'), type: 'badge', status: props.status.enabled ? 'ok' : 'inactive' },
  ].filter(Boolean);
});

async function serviceAction(action) {
  pendingAction.value = action;
  try {
    await api.post(`/api/smb/service/${action}`, {});
    toast.toast(t('smb.serviceActionDone', { action }));
    emit('refresh');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    pendingAction.value = '';
  }
}

async function toggleEnable() {
  const enable = !props.status?.enabled;
  try {
    await api.post('/api/smb/service/reload', { enable });
    toast.toast(enable ? t('smb.serviceEnabled') : t('smb.serviceDisabled'));
    emit('refresh');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}
</script>

<template>
  <StatusBar :items="items">
    <template #actions>
      <button class="btn-sm" :class="{ 'btn-secondary': serviceRunning }" :disabled="pendingAction || serviceRunning" @click="serviceAction('start')">
        <i class="fa-solid fa-play"></i> {{ t('common.start') }}
      </button>
      <button class="btn-sm btn-secondary" :disabled="pendingAction || !serviceRunning" @click="serviceAction('stop')">
        <i class="fa-solid fa-stop"></i> {{ t('common.stop') }}
      </button>
      <button class="btn-sm btn-secondary" :disabled="pendingAction" @click="serviceAction('restart')">
        <i class="fa-solid fa-rotate-right"></i> {{ t('common.restart') }}
      </button>
      <button class="btn-sm" :class="status?.enabled ? 'btn-secondary' : ''" @click="toggleEnable">
        <i :class="status?.enabled ? 'fa-regular fa-square-check' : 'fa-regular fa-square'"></i>
        {{ t('smb.autoStart') }}
      </button>
    </template>
  </StatusBar>
</template>
