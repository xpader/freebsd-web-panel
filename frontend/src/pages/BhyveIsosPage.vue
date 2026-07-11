<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const isos = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');

const showFetch = ref(false);
const fetchUrl = ref('');
const fetching = ref(false);

function openFetch() {
  fetchUrl.value = '';
  showFetch.value = true;
}

async function submitFetch() {
  if (!fetchUrl.value.trim()) return;
  fetching.value = true;
  try {
    await api.post('/api/bhyve/isos', { url: fetchUrl.value.trim() });
    toast.toast(t('bhyve.fetchIsoDone'));
    showFetch.value = false;
    await load();
  } catch (e) {
    await alert(t('bhyve.fetchIsoFailed'), e.message || t('bhyve.fetchIsoFailed'));
  } finally {
    fetching.value = false;
  }
}

async function load() {
  if (!isos.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    isos.value = await api.get('/api/bhyve/isos');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function removeIso(iso) {
  if (!await confirm(t('bhyve.deleteIsoTitle'), t('bhyve.deleteIsoConfirm', { name: iso.name }))) return;
  try {
    await api.del(`/api/bhyve/isos/${encodeURIComponent(iso.name)}`);
    toast.toast(t('bhyve.isoDeleted', { name: iso.name }));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>ISO</h1>
    <p>{{ t('bhyve.isoSubtitle') }}</p>
  </div>

  <div class="toolbar">
    <div></div>
    <div class="flex btn-group">
      <button @click="openFetch"><i class="fa-solid fa-download"></i> {{ t('bhyve.fetchIso') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('common.size') }}</th>
        <th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="3" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="3" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!isos.length"><td colspan="3" class="empty">{{ t('bhyve.noIsos') }}</td></tr>
        <tr v-for="iso in isos" :key="iso.name">
          <td class="mono"><strong>{{ iso.name }}</strong></td>
          <td>{{ fmtBytes(iso.size) }}</td>
          <td>
            <button class="btn-danger btn-sm" @click="removeIso(iso)">{{ t('common.delete') }}</button>
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Download ISO modal -->
  <div v-if="showFetch" class="modal-overlay">
    <div class="modal" style="max-width:560px;">
      <h3>{{ t('bhyve.fetchIso') }}</h3>
      <form @submit.prevent="submitFetch">
        <div class="field">
          <label>URL <span style="color:var(--danger)">*</span></label>
          <input type="url" v-model="fetchUrl" required :placeholder="t('bhyve.fetchIsoUrlPlaceholder')" :disabled="fetching" />
          <p class="field-hint">{{ t('bhyve.fetchIsoUrlHint') }}</p>
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showFetch = false" :disabled="fetching">{{ t('common.cancel') }}</button>
          <button type="submit" :disabled="fetching">
            <i v-if="fetching" class="fa-solid fa-spinner fa-spin"></i>
            {{ fetching ? t('bhyve.fetchIsoRunning') : t('bhyve.fetchIso') }}
          </button>
        </div>
      </form>
    </div>
  </div>
</template>
