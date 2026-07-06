<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();

const allServices = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.toLowerCase();
  if (!q) return allServices.value;
  return allServices.value.filter((s) =>
    s.name.toLowerCase().includes(q) || (s.description || '').toLowerCase().includes(q)
  );
});

async function load() {
  if (!allServices.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    allServices.value = await api.get('/api/services');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function action(name, act) {
  try {
    await api.post(`/api/services/${encodeURIComponent(name)}/${act}`);
    toast.toast(t('svc.actionDone', { name, action: t('svc.' + act) }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('svc.title') }}</h1>
    <p>{{ t('svc.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <input type="text" v-model="filter" class="filter-input" :placeholder="t('svc.filter')" />
    <span class="text-dim">{{ t('svc.count', { n: filtered.length }) }}</span>
    <div class="flex">
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('svc.location') }}</th>
        <th>{{ t('common.description') }}</th>
        <th>{{ t('common.enabled') }}</th>
        <th>{{ t('common.status') }}</th>
        <th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="6" class="empty">{{ t('svc.noServices') }}</td></tr>
        <tr v-for="s in filtered" :key="s.name">
          <td class="mono"><strong>{{ s.name }}</strong></td>
          <td><span :class="['badge', s.source === 'system' ? 'badge-dim' : '']">{{ s.source === 'system' ? t('svc.system') : t('svc.local') }}</span></td>
          <td><div class="cell-wrap">{{ s.description || '—' }}</div></td>
          <td>
            <span v-if="s.enabled" class="badge badge-success">{{ t('common.enabled') }}</span>
            <span v-else class="badge badge-dim">{{ t('common.disabled') }}</span>
          </td>
          <td>
            <span v-if="s.running" class="badge badge-success">{{ t('svc.running') }}</span>
            <span v-else-if="s.enabled" class="badge badge-warn">{{ t('svc.stopped') }}</span>
            <span v-else class="badge badge-dim">{{ t('svc.stopped') }}</span>
          </td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" :disabled="s.running" @click="action(s.name, 'start')">{{ t('svc.start') }}</button>
              <button class="btn-secondary btn-sm" :disabled="!s.running" @click="action(s.name, 'stop')">{{ t('svc.stop') }}</button>
              <button class="btn-secondary btn-sm" @click="action(s.name, 'restart')">{{ t('svc.restart') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
