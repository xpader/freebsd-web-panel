<script setup>
import { computed, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';
import { useToast, useAlert, useConfirm, useCodePreview, useCountdown } from '../../composables/useDialog.js';
import { ui } from '../../stores/ui.js';
import StatusBar from './StatusBar.vue';

const props = defineProps({
  status: { type: Object, required: true },
});

const emit = defineEmits(['refresh', 'status', 'discarded']);

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const codePreview = useCodePreview();
const countdown = useCountdown();

const items = computed(() => [
  { title: t('firewall.driver'), value: props.status.driver },
  { title: t('firewall.mode'), value: props.status.mode === 'whitelist' ? t('firewall.whitelist') : t('firewall.blacklist') },
  { title: t('common.status'), value: props.status.enabled ? t('common.running') : t('common.stopped'), type: 'badge', status: props.status.enabled ? 'ok' : 'inactive' },
  { title: t('firewall.ruleCount'), value: props.status.rules_count },
  ...(props.status.pending_apply ? [{ title: t('firewall.pendingApply'), value: t('common.yes'), type: 'badge', status: 'warning' }] : []),
]);

async function refresh() {
  emit('refresh');
}

async function showCountdownIfPending(response) {
  const pending = response?.pending_confirm;
  if (!pending) return;
  const isEnable = pending.operation === 'enable';
  const action = await countdown(
    isEnable ? t('firewall.confirmTitleEnable') : t('firewall.confirmTitle'),
    isEnable ? t('firewall.confirmMessageEnable') : t('firewall.confirmMessage'),
    pending.expires_at,
    pending.timeout_seconds,
    {
      rollbackLabel: isEnable ? t('firewall.rollbackNowEnable') : t('firewall.rollbackNowApply'),
      confirmLabel: isEnable ? t('firewall.keepChangesEnable') : t('firewall.keepChangesApply'),
      probeUrl: '/api/firewall/status',
      warningMessage: t('firewall.serverUnreachable'),
    },
  );
  try {
    const response = await api.post(action === 'confirm' ? '/api/firewall/confirm' : '/api/firewall/rollback');
    emit('status', response);
    toast.toast(action === 'confirm' ? t('firewall.confirmed') : t('firewall.rolledBack'));
    await refresh();
  } catch (error) {
    await alert(t('common.operationFailed'), error.message || t('common.operationFailed'));
    await refresh();
  }
}

async function doApply() {
  try {
    const response = await api.post('/api/firewall/apply');
    emit('status', { ...props.status, pending_apply: false });
    if (response.pending_confirm) await showCountdownIfPending(response);
    else toast.toast(t('firewall.applied'));
    await refresh();
  } catch (error) {
    try {
      const status = await api.get('/api/firewall/status');
      emit('status', status);
      if (status.pending_confirm) {
        await showCountdownIfPending(status);
        return;
      }
    } catch (_) {}
    await alert(t('firewall.applyFailed'), error.message || t('firewall.applyFailed'));
  }
}

async function doDiscard() {
  if (!await confirm(t('firewall.discard'), t('firewall.discardConfirm'))) return;
  try {
    await api.post('/api/firewall/discard');
    toast.toast(t('firewall.discarded'));
    emit('discarded');
    await refresh();
  } catch (error) {
    await alert(t('common.operationFailed'), error.message || t('common.operationFailed'));
  }
}

async function doToggleEnabled() {
  if (props.status.enabled && !await confirm(t('firewall.disableTitle'), t('firewall.disableConfirm'))) return;
  try {
    const response = await api.post(props.status.enabled ? '/api/firewall/disable' : '/api/firewall/enable');
    emit('status', response);
    if (response.pending_confirm) await showCountdownIfPending(response);
    else toast.toast(response.enabled ? t('firewall.enabled') : t('firewall.disabled'));
    await refresh();
  } catch (error) {
    await alert(t('common.operationFailed'), error.message || t('common.operationFailed'));
  }
}

async function showConfigPreview() {
  try {
    const data = await api.get('/api/firewall/config');
    codePreview(t('firewall.configPreview'), data.content);
  } catch (error) {
    await alert(t('common.operationFailed'), error.message || t('common.operationFailed'));
  }
}

let pollTimer = null;
onMounted(() => {
  pollTimer = setInterval(async () => {
    if (!props.status?.pending_confirm || ui.dialog) return;
    try {
      const status = await api.get('/api/firewall/status');
      emit('status', status);
      if (status.pending_confirm) await showCountdownIfPending(status);
    } catch (_) {}
  }, 5000);
});
onUnmounted(() => clearInterval(pollTimer));
</script>

<template>
  <StatusBar :items="items">
    <template #actions>
      <button v-if="status.pending_apply" class="btn-sm" @click="doApply">
        <i class="fa-solid fa-check"></i> {{ t('firewall.applyRules') }}
      </button>
      <button v-if="status.pending_apply" class="btn-secondary" @click="doDiscard">
        <i class="fa-solid fa-rotate-left"></i> {{ t('firewall.discard') }}
      </button>
      <button :class="['btn-sm', status.enabled ? 'btn-danger' : '']" @click="doToggleEnabled">
        <i :class="status.enabled ? 'fa-solid fa-stop' : 'fa-solid fa-play'"></i>
        {{ status.enabled ? t('common.stop') : t('common.start') }}
      </button>
      <button class="btn-secondary btn-sm" @click="showConfigPreview">
        <i class="fa-solid fa-eye"></i> {{ t('firewall.configPreview') }}
      </button>
      <button class="btn-secondary btn-sm" @click="refresh">
        <i class="fa-solid fa-rotate"></i> {{ t('common.refresh') }}
      </button>
    </template>
  </StatusBar>
</template>
