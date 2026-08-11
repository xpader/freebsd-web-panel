<script setup>
import { ref, computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { api } from '../../lib/api.js';
import { useToast, useAlert } from '../../composables/useDialog.js';
import StatusBar from './StatusBar.vue';

const props = defineProps({
  status: { type: Object, required: true },
});

const emit = defineEmits(['refresh']);
const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const pendingAction = ref('');

const serviceRunning = computed(() => props.status?.service_running ?? false);

const items = computed(() => {
  if (!props.status) return [];
  return [
    { title: t('common.status'), value: serviceRunning.value ? t('common.running') : t('common.stopped'), type: 'badge', status: serviceRunning.value ? 'ok' : 'inactive' },
    ...(props.status.version ? [{ title: t('smb.version'), value: `Samba ${props.status.version}` }] : []),
  ].filter(Boolean);
});

const actionLabels = { start: 'common.start', stop: 'common.stop', restart: 'common.restart' };

async function serviceAction(action) {
  pendingAction.value = action;
  try {
    const res = await api.post(`/api/smb/service/${action}`, {});
    if (res.firewall_needs_reload) {
      const goFirewall = await alert(t('smb.firewallReloadTitle'), t('smb.firewallReloadMsg'), {
        dismissLabel: t('common.gotIt'),
        actionLabel: t('smb.goToFirewall'),
      });
      if (goFirewall) {
        router.push('/firewall/rules');
      }
    }
    emit('refresh');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    pendingAction.value = '';
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
      <button class="btn-sm btn-secondary" :disabled="!!pendingAction || !serviceRunning" @click="serviceAction('restart')">
        <i class="fa-solid fa-rotate-right"></i> {{ t('common.restart') }}
      </button>
    </template>
  </StatusBar>
</template>
