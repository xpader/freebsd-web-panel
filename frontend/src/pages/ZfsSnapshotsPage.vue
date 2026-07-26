<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes, fmtTime } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import SearchInput from '../components/ui/SearchInput.vue';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const allSnaps = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.toLowerCase();
  if (!q) return allSnaps.value;
  return allSnaps.value.filter((s) => s.dataset.toLowerCase().includes(q) || s.snap_name.toLowerCase().includes(q));
});

async function load() {
  if (!allSnaps.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    allSnaps.value = await api.get('/api/zfs/snapshots');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function createSnap() {
  const result = await formModal(t('zfs.snapCreateTitle'), [
    { key: 'dataset', label: t('zfs.dsLabel'), placeholder: 'zroot/data', required: true },
    { key: 'name', label: t('zfs.snapNameLabel'), placeholder: t('zfs.snapNamePlaceholder'), required: true },
  ]);
  if (!result) return;
  try {
    await api.post('/api/zfs/snapshots', { dataset: result.dataset, name: result.name });
    toast.toast(t('zfs.snapCreatedShort'));
    await load();
  } catch (e) {
    await alert(t('zfs.snapCreateFailedShort'), e.message || t('zfs.snapCreateFailedShort'));
  }
}

async function cloneSnap(source) {
  const result = await formModal(t('zfs.cloneTitle', { name: source }), [
    { key: 'target', label: t('zfs.cloneTargetLabel'), placeholder: t('zfs.cloneTargetPlaceholder'), required: true },
    { key: 'mountpoint', label: t('zfs.cloneMountpointLabel'), placeholder: t('zfs.cloneMountpointPlaceholder') },
  ]);
  if (!result) return;
  try {
    await api.post('/api/zfs/snapshot/clone', { source, target: result.target, mountpoint: result.mountpoint || undefined });
    toast.toast(t('zfs.cloneDone', { name: result.target }));
    await load();
  } catch (e) {
    await alert(t('zfs.cloneFailed'), e.message || t('zfs.cloneFailed'));
  }
}

async function delSnap(full) {
  const result = await confirm(t('zfs.snapDeleteTitle'), t('zfs.snapDeleteConfirm', { name: full }), [
    { key: 'recursive', label: t('zfs.snapRecursive'), checked: false },
  ]);
  if (!result || !result.confirmed) return;
  const qs = `name=${encodeURIComponent(full)}${result.recursive ? '&recursive=true' : ''}`;
  try {
    await api.del(`/api/zfs/snapshot/destroy?${qs}`);
    toast.toast(t('zfs.snapDeleted'));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

async function rollbackSnap(full) {
  if (!await confirm(t('zfs.snapRollbackTitle'), t('zfs.snapRollbackConfirm', { name: full }))) return;
  try {
    await api.post(`/api/zfs/snapshot/rollback?name=${encodeURIComponent(full)}`, { confirm: true });
    toast.toast(t('zfs.snapRollbackDone'));
    await load();
  } catch (e) {
    await alert(t('zfs.snapRollbackFailed'), e.message || t('zfs.snapRollbackFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('zfs.snapTitle') }}</h1>
    <p>{{ t('zfs.snapSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <SearchInput v-model="filter" :placeholder="t('zfs.snapFilter')" />
    <div class="flex">
      <button @click="createSnap"><i class="fa-solid fa-plus"></i> {{ t('zfs.snapCreate') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('zfs.dsLabel') }}</th><th>{{ t('zfs.snapshot') }}</th><th>{{ t('common.used') }}</th>
        <th>{{ t('zfs.refer') }}</th><th>{{ t('common.createdAt') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="6" class="empty">{{ t('zfs.noSnaps') }}</td></tr>
        <tr v-for="s in filtered" :key="s.name">
          <td class="mono">{{ s.dataset }}</td>
          <td class="mono">{{ s.snap_name }}</td>
          <td class="mono">{{ fmtBytes(s.used) }}</td>
          <td class="mono">{{ fmtBytes(s.referenced) }}</td>
          <td class="text-dim mono">{{ fmtTime(s.creation) }}</td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="cloneSnap(s.name)">{{ t('zfs.clone') }}</button>
              <button class="btn-secondary btn-sm" @click="rollbackSnap(s.name)">{{ t('zfs.rollback') }}</button>
              <button class="btn-danger btn-sm" @click="delSnap(s.name)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
