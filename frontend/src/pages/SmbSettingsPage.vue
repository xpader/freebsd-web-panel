<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import SmbStatusBar from '../components/shared/SmbStatusBar.vue';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();

const config = ref(null);
const status = ref(null);
const loading = ref(true);
const saving = ref(false);

const protocolOptions = [
  { value: 'SMB2', label: 'SMB2' },
  { value: 'SMB3', label: 'SMB3' },
  { value: 'NT1', label: 'SMBv1 (NT1)' },
];

const guestOptions = [
  { value: 'Never', label: t('smb.guestNever') },
  { value: 'Bad User', label: t('smb.guestBadUser') },
  { value: 'Bad Password', label: t('smb.guestBadPassword') },
];

async function load() {
  loading.value = true;
  try {
    const [cfg, st] = await Promise.all([
      api.get('/api/smb/config'),
      api.get('/api/smb/status'),
    ]);
    config.value = cfg;
    status.value = st;
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    loading.value = false;
  }
}

async function saveConfig() {
  saving.value = true;
  try {
    await api.put('/api/smb/config', config.value);
    toast.toast(t('smb.configSaved'));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('smb.settings') }}</h1>
    <p>{{ t('smb.settingsSubtitle') }}</p>
  </div>

  <div v-if="loading" class="card" style="text-align:center;padding:24px;">
    <span class="spinner"></span> {{ t('common.loading') }}
  </div>

  <template v-else-if="config">
    <SmbStatusBar :status="status" @refresh="load" />

    <div class="card">
      <h3>{{ t('smb.globalConfig') }}</h3>
      <div class="form-grid">
        <div class="field">
          <label>{{ t('smb.workgroup') }}</label>
          <input v-model="config.workgroup" />
        </div>
        <div class="field">
          <label>{{ t('smb.serverString') }}</label>
          <input v-model="config.server_string" />
        </div>
        <div class="field">
          <label>{{ t('smb.minProtocol') }}</label>
          <select v-model="config.server_min_protocol">
            <option v-for="o in protocolOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
          </select>
        </div>
        <div class="field">
          <label>{{ t('smb.mapToGuest') }}</label>
          <select v-model="config.map_to_guest">
            <option v-for="o in guestOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
          </select>
        </div>
        <div class="field">
          <label>{{ t('smb.logLevel') }}</label>
          <input type="number" v-model.number="config.log_level" min="0" max="10" />
        </div>
      </div>
      <div class="btn-group" style="margin-top:16px;">
        <button @click="saveConfig" :disabled="saving">
          <i class="fa-solid fa-floppy-disk"></i> {{ t('common.save') }}
        </button>
      </div>
    </div>
  </template>
</template>
