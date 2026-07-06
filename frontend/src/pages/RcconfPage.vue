<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const allVars = ref([]);
const loading = ref(true);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.toLowerCase();
  if (!q) return allVars.value;
  return allVars.value.filter((v) => v.key.toLowerCase().includes(q) || v.value.toLowerCase().includes(q));
});

async function load() {
  if (!allVars.value.length) loading.value = true;
  error.value = '';
  try {
    allVars.value = await api.get('/api/rcconf');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
}

async function doAdd() {
  const result = await formModal(t('rcconf.addTitle'), [
    { key: 'key', label: t('common.key'), placeholder: t('rcconf.keyPlaceholder'), required: true },
    { key: 'value', label: t('common.value'), placeholder: 'YES' },
  ], t('rcconf.add'));
  if (!result) return;
  try {
    await api.put('/api/rcconf', { key: result.key.trim(), value: result.value });
    toast.toast(t('rcconf.added'));
    await load();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  }
}

async function doEdit(v) {
  const result = await formModal(t('rcconf.editTitle', { key: v.key }), [
    { key: 'value', label: t('common.value'), value: v.value || '', placeholder: 'YES' },
  ], t('common.save'));
  if (!result) return;
  try {
    await api.put('/api/rcconf', { key: v.key, value: result.value });
    toast.toast(t('rcconf.saved'));
    await load();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  }
}

async function doDelete(v) {
  if (!await confirm(t('rcconf.deleteTitle'), t('rcconf.deleteConfirm', { key: v.key }))) return;
  try {
    await api.del(`/api/rcconf?key=${encodeURIComponent(v.key)}`);
    toast.toast(t('rcconf.deleted'));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('rcconf.title') }}</h1>
    <p>{{ t('rcconf.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <input type="text" v-model="filter" class="filter-input" :placeholder="t('rcconf.filter')" />
    <span class="text-dim">{{ t('rcconf.count', { n: filtered.length }) }}</span>
    <div class="flex">
      <button @click="doAdd"><i class="fa-solid fa-plus"></i> {{ t('rcconf.add') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr><th>{{ t('common.key') }}</th><th>{{ t('common.value') }}</th><th>{{ t('common.actions') }}</th></tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="3" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="3" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="3" class="empty">{{ t('rcconf.noVars') }}</td></tr>
        <tr v-for="v in filtered" :key="v.key">
          <td class="mono"><strong>{{ v.key }}</strong></td>
          <td class="mono"><div class="cell-wrap">{{ v.value || '—' }}</div></td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="doEdit(v)">{{ t('common.edit') }}</button>
              <button class="btn-danger btn-sm" @click="doDelete(v)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
