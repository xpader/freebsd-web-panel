<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const datastores = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');

async function load() {
  if (!datastores.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    datastores.value = await api.get('/api/bhyve/datastores');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function addDatastore() {
  const result = await formModal(
    t('bhyve.dsCreateTitle'),
    [
      { key: 'name', label: t('common.name'), placeholder: t('bhyve.dsNamePlaceholder'), required: true },
      {
        key: 'type',
        label: t('common.type'),
        type: 'select',
        required: true,
        options: [
          { value: 'zfs', label: 'ZFS' },
          { value: 'directory', label: t('bhyve.dsTypeDirectory') },
          { value: 'iso', label: 'ISO' },
          { value: 'img', label: 'IMG' },
        ],
      },
      { key: 'path', label: t('bhyve.dsPathOrDataset'), placeholder: 'zroot/vm-data', required: true },
    ],
    t('common.create'),
  );
  if (!result) return;

  let spec;
  if (result.type === 'zfs') {
    spec = `zfs:${result.path}`;
  } else if (result.type === 'iso' || result.type === 'img') {
    spec = `${result.type}:${result.path}`;
  } else {
    spec = result.path;
  }

  try {
    await api.post('/api/bhyve/datastores', { name: result.name, spec });
    toast.toast(t('bhyve.dsCreated', { name: result.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function removeDatastore(ds) {
  if (!await confirm(t('bhyve.dsDeleteTitle'), t('bhyve.dsDeleteConfirm', { name: ds.name }))) return;
  try {
    await api.del(`/api/bhyve/datastores/${encodeURIComponent(ds.name)}`);
    toast.toast(t('bhyve.dsDeleted', { name: ds.name }));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.tabDatastores') }}</h1>
    <p>{{ t('bhyve.dsSubtitle') }}</p>
  </div>

  <div class="toolbar">
    <div></div>
    <div class="flex btn-group">
      <button @click="addDatastore"><i class="fa-solid fa-plus"></i> {{ t('common.create') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('common.type') }}</th>
        <th>{{ t('bhyve.dsPath') }}</th>
        <th>{{ t('bhyve.dsZfsDataset') }}</th>
        <th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="5" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="5" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!datastores.length"><td colspan="5" class="empty">{{ t('bhyve.noDatastores') }}</td></tr>
        <tr v-for="ds in datastores" :key="ds.name">
          <td class="mono"><strong>{{ ds.name }}</strong></td>
          <td><span class="badge badge-dim">{{ ds.type }}</span></td>
          <td class="mono">{{ ds.path }}</td>
          <td class="mono">{{ ds.zfs_dataset || '—' }}</td>
          <td>
            <button v-if="ds.name !== 'default'" class="btn-danger btn-sm" @click="removeDatastore(ds)">{{ t('common.delete') }}</button>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
