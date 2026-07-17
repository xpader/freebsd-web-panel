<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal, useCountdown } from '../composables/useDialog.js';
import { ui } from '../stores/ui.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();
const countdown = useCountdown();

const status = ref(null);
const loading = ref(true);

const initialized = computed(() => status.value?.initialized);

async function loadStatus() {
  try {
    status.value = await api.get('/api/firewall/status');
  } catch (e) {
    if (!status.value) loading.value = false;
    return;
  }
  loading.value = false;
}

async function doInitialize() {
  const result = await formModal(t('firewall.initTitle'), [
    {
      key: 'driver',
      label: t('firewall.driver'),
      type: 'radio',
      options: [
        { value: 'ipfw', label: 'ipfw' },
        { value: 'pf', label: 'pf' },
      ],
      value: 'ipfw',
    },
    {
      key: 'mode',
      label: t('firewall.mode'),
      type: 'radio',
      options: [
        { value: 'whitelist', label: t('firewall.whitelist') },
        { value: 'blacklist', label: t('firewall.blacklist') },
      ],
      value: 'blacklist',
    },
  ], { submitLabel: t('firewall.initialize') });

  if (!result) return;

  try {
    await api.post('/api/firewall/initialize', { driver: result.driver, mode: result.mode });
    toast.toast(t('firewall.initialized'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doToggleEnabled() {
  if (status.value.enabled) {
    if (!await confirm(t('firewall.disableTitle'), t('firewall.disableConfirm'))) return;
    try {
      status.value = await api.post('/api/firewall/disable');
      toast.toast(t('firewall.disabled'));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
  } else {
    try {
      const resp = await api.post('/api/firewall/enable');
      status.value = resp;
      if (resp.pending_confirm) {
        await showCountdownIfPending(resp);
      } else {
        toast.toast(t('firewall.enabled'));
      }
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
  }
}

async function showCountdownIfPending(resp) {
  const pc = resp?.pending_confirm;
  if (!pc) return;
  const action = await countdown(
    t('firewall.confirmTitle'),
    t('firewall.confirmMessage'),
    pc.expires_at,
    pc.timeout_seconds,
  );
  if (action === 'confirm') {
    try {
      status.value = await api.post('/api/firewall/confirm');
      toast.toast(t('firewall.confirmed'));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
      await loadStatus();
    }
  } else {
    try {
      status.value = await api.post('/api/firewall/rollback');
      toast.toast(t('firewall.rolledBack'));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
      await loadStatus();
    }
  }
}

async function doSwitchDriver(newDriver) {
  if (newDriver === status.value.driver) return;

  if (!await confirm(t('firewall.switchConfirmTitle'),
      t('firewall.switchConfirm', { from: status.value.driver, to: newDriver }))) return;
  try {
    status.value = await api.post('/api/firewall/switch', { driver: newDriver });
    toast.toast(t('firewall.switched', { driver: newDriver }));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doSwitchMode(newMode) {
  if (newMode === status.value.mode) return;

  const warnMsg = newMode === 'whitelist'
    ? t('firewall.modeSwitchWhitelistWarn')
    : t('firewall.modeSwitchBlacklistWarn');
  if (!await confirm(t('firewall.modeConfirmTitle'), warnMsg)) return;
  try {
    status.value = await api.put('/api/firewall/mode', { mode: newMode });
    toast.toast(t('firewall.modeSwitched'));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

let pollTimer = null;
onMounted(() => {
  loadStatus();
  pollTimer = setInterval(async () => {
    // Only poll when there's an active pending confirmation (countdown timer running).
    if (status.value?.pending_confirm && !ui.dialog) {
      await loadStatus();
      if (status.value?.pending_confirm) {
        showCountdownIfPending(status.value);
      }
    }
  }, 5000);
});
onUnmounted(() => clearInterval(pollTimer));
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <h1>{{ t('nav.firewallSettings') }}</h1>
      <p class="text-dim" style="margin:0;font-size:13px;">{{ t('firewall.subtitle') }}</p>
    </div>
    <div v-if="initialized" class="flex btn-group" style="margin-left:auto;">
      <button :class="status.enabled ? 'btn-danger' : ''" @click="doToggleEnabled">
        <i :class="status.enabled ? 'fa-solid fa-stop' : 'fa-solid fa-play'"></i>
        {{ status.enabled ? t('common.stop') : t('common.start') }}
      </button>
    </div>
  </div>

  <div v-if="loading" class="card">
    <div class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  </div>

  <template v-else-if="!initialized">
    <div class="card firewall-init">
      <h3>{{ t('firewall.initTitle') }}</h3>
      <p class="text-dim">{{ t('firewall.initDesc') }}</p>
      <div class="firewall-driver-compare">
        <div class="driver-card">
          <h4>ipfw</h4>
          <ul class="text-dim">
            <li>{{ t('firewall.ipfwFeature1') }}</li>
            <li>{{ t('firewall.ipfwFeature2') }}</li>
            <li>{{ t('firewall.ipfwFeature3') }}</li>
          </ul>
        </div>
        <div class="driver-card">
          <h4>pf</h4>
          <ul class="text-dim">
            <li>{{ t('firewall.pfFeature1') }}</li>
            <li>{{ t('firewall.pfFeature2') }}</li>
            <li>{{ t('firewall.pfFeature3') }}</li>
          </ul>
        </div>
      </div>
      <div class="modal-actions" style="justify-content:center;">
        <button @click="doInitialize"><i class="fa-solid fa-shield-halved"></i> {{ t('firewall.initialize') }}</button>
      </div>
    </div>
  </template>

  <template v-else>
    <div class="fw-settings-grid">
      <div class="card">
        <h3 style="margin:0 0 16px 0;">{{ t('firewall.driver') }}</h3>
        <div class="fw-radio-group">
          <div
            class="fw-radio-item"
            :class="{ active: status.driver === 'ipfw' }"
            @click="doSwitchDriver('ipfw')"
          >
            <div class="fw-radio-indicator">
              <i v-if="status.driver === 'ipfw'" class="fa-solid fa-circle-check"></i>
              <i v-else class="fa-regular fa-circle"></i>
            </div>
            <div class="fw-radio-content">
              <strong>ipfw</strong>
              <ul class="text-dim" style="margin:4px 0 0 0;padding-left:16px;font-size:12px;line-height:1.6;">
                <li>{{ t('firewall.ipfwFeature1') }}</li>
                <li>{{ t('firewall.ipfwFeature2') }}</li>
              </ul>
            </div>
          </div>
          <div
            class="fw-radio-item"
            :class="{ active: status.driver === 'pf' }"
            @click="doSwitchDriver('pf')"
          >
            <div class="fw-radio-indicator">
              <i v-if="status.driver === 'pf'" class="fa-solid fa-circle-check"></i>
              <i v-else class="fa-regular fa-circle"></i>
            </div>
            <div class="fw-radio-content">
              <strong>pf</strong>
              <ul class="text-dim" style="margin:4px 0 0 0;padding-left:16px;font-size:12px;line-height:1.6;">
                <li>{{ t('firewall.pfFeature1') }}</li>
                <li>{{ t('firewall.pfFeature2') }}</li>
              </ul>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <h3 style="margin:0 0 16px 0;">{{ t('firewall.mode') }}</h3>
        <div class="fw-radio-group">
          <div
            class="fw-radio-item"
            :class="{ active: status.mode === 'whitelist' }"
            @click="doSwitchMode('whitelist')"
          >
            <div class="fw-radio-indicator">
              <i v-if="status.mode === 'whitelist'" class="fa-solid fa-circle-check"></i>
              <i v-else class="fa-regular fa-circle"></i>
            </div>
            <div class="fw-radio-content">
              <strong>{{ t('firewall.whitelist') }}</strong>
              <p class="text-dim" style="margin:4px 0 0 0;font-size:12px;line-height:1.6;">{{ t('firewall.modeSwitchWhitelistWarn') }}</p>
            </div>
          </div>
          <div
            class="fw-radio-item"
            :class="{ active: status.mode === 'blacklist' }"
            @click="doSwitchMode('blacklist')"
          >
            <div class="fw-radio-indicator">
              <i v-if="status.mode === 'blacklist'" class="fa-solid fa-circle-check"></i>
              <i v-else class="fa-regular fa-circle"></i>
            </div>
            <div class="fw-radio-content">
              <strong>{{ t('firewall.blacklist') }}</strong>
              <p class="text-dim" style="margin:4px 0 0 0;font-size:12px;line-height:1.6;">{{ t('firewall.modeSwitchBlacklistWarn') }}</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  </template>
</template>

<style scoped>
.firewall-init {
  text-align: center;
  padding: 32px;
}
.firewall-init h3 {
  margin-bottom: 8px;
}
.firewall-init > p {
  margin-bottom: 24px;
}
.firewall-driver-compare {
  display: flex;
  gap: 16px;
  margin-bottom: 24px;
}
.driver-card {
  flex: 1;
  text-align: left;
  padding: 16px;
  border-radius: var(--radius);
  background: var(--bg-elev2);
}
.driver-card h4 {
  margin: 0 0 8px 0;
}
.driver-card ul {
  margin: 0;
  padding-left: 20px;
  font-size: 13px;
  line-height: 1.8;
}
.fw-settings-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}
@media (max-width: 768px) {
  .fw-settings-grid {
    grid-template-columns: 1fr;
  }
}
.fw-radio-group {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.fw-radio-item {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 16px;
  border-radius: var(--radius);
  background: var(--bg-elev2);
  border: 1px solid transparent;
  transition: border-color 0.15s, background 0.15s;
  cursor: pointer;
}
.fw-radio-item:hover {
  background: var(--bg-elev3);
}
.fw-radio-item.active {
  border-color: var(--accent);
}
.fw-radio-indicator {
  font-size: 18px;
  margin-top: 2px;
  color: var(--text-dim);
}
.fw-radio-item.active .fw-radio-indicator {
  color: var(--accent);
}
.fw-radio-content {
  flex: 1;
}
</style>
