<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const tree = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const flatRows = ref([]);
const showPropsFor = ref(null);
const propsData = ref(null);

function walk(ds, depth, rows) {
  rows.push({ ...ds, depth });
  if (ds.children) ds.children.forEach((c) => walk(c, depth + 1, rows));
}

async function load() {
  if (!flatRows.value.length) loading.value = true;
  error.value = '';
  try {
    tree.value = await api.get('/api/zfs/datasets');
    flatRows.value = [];
    tree.value.forEach((ds) => walk(ds, 0, flatRows.value));
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function createDataset() {
  const result = await formModal(t('zfs.dsCreateTitle'), [
    { key: 'name', label: t('zfs.dsNameLabel'), placeholder: t('zfs.dsNamePlaceholder'), required: true },
  ]);
  if (!result) return;
  try {
    await api.post('/api/zfs/datasets', { name: result.name });
    toast.toast(t('zfs.dsCreated'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function snapshotDataset(name) {
  const result = await formModal(t('zfs.dsCreateSnapTitle', { name }), [
    { key: 'name', label: t('zfs.snapNameLabel'), placeholder: t('zfs.snapNamePlaceholder'), required: true },
  ]);
  if (!result) return;
  try {
    await api.post('/api/zfs/snapshots', { dataset: name, name: result.name });
    toast.toast(t('zfs.snapCreated', { name: `${name}@${result.name}` }));
  } catch (e) {
    await alert(t('zfs.snapCreateFailed'), e.message || t('zfs.snapCreateFailed'));
  }
}

async function deleteDataset(name) {
  if (!await confirm(t('zfs.dsDeleteTitle'), t('zfs.dsDeleteConfirm', { name }))) return;
  try {
    await api.del(`/api/zfs/dataset/destroy?name=${encodeURIComponent(name)}`);
    toast.toast(t('zfs.dsDeleted'));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

async function showProps(name) {
  try {
    propsData.value = await api.get(`/api/zfs/dataset/properties?name=${encodeURIComponent(name)}`);
    showPropsFor.value = name;
  } catch (e) {
    await alert(t('zfs.dsPropsFailed'), e.message || t('zfs.dsPropsFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('zfs.dsTitle') }}</h1>
    <p>{{ t('zfs.dsSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <div></div>
    <div class="flex">
      <button @click="createDataset"><i class="fa-solid fa-plus"></i> {{ t('zfs.dsCreate') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th><th>{{ t('common.type') }}</th><th>{{ t('common.used') }}</th>
        <th>{{ t('common.available') }}</th><th>{{ t('zfs.mountpoint') }}</th><th>{{ t('zfs.compression') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="7" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="7" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!flatRows.length"><td colspan="7" class="empty">{{ t('zfs.noDatasets') }}</td></tr>
        <tr v-for="(ds, i) in flatRows" :key="i">
          <td class="mono" :style="{ paddingLeft: ds.depth * 20 + 12 + 'px' }">
            {{ ds.depth > 0 ? '└ ' : '' }}<strong>{{ ds.name }}</strong>
            <div v-if="ds.origin" class="text-dim" style="font-size:11px;margin-top:2px;">
              <i class="fa-solid fa-code-branch"></i> {{ t('zfs.clonedFrom') }} <span class="mono" style="color:var(--accent);">{{ ds.origin }}</span>
            </div>
          </td>
          <td><span class="badge badge-dim">{{ ds.typ }}</span></td>
          <td class="mono">{{ fmtBytes(ds.used) }}</td>
          <td class="mono">{{ fmtBytes(ds.available) }}</td>
          <td class="mono">{{ ds.mountpoint }}</td>
          <td class="mono">{{ ds.compression }}</td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="snapshotDataset(ds.name)">{{ t('zfs.snapshot') }}</button>
              <button class="btn-secondary btn-sm" @click="showProps(ds.name)">{{ t('zfs.properties') }}</button>
              <button v-if="ds.name.includes('/')" class="btn-danger btn-sm" @click="deleteDataset(ds.name)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Properties modal -->
  <div v-if="showPropsFor" class="modal-overlay">
    <div class="modal" style="max-width:600px;">
      <h3>{{ t('zfs.propsTitle', { name: showPropsFor }) }}</h3>
      <div style="max-height:400px;overflow-y:auto;">
        <table style="font-size:12px;">
          <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('common.value') }}</th><th>{{ t('zfs.source') }}</th></tr></thead>
          <tbody>
            <tr v-for="(p, i) in (propsData || [])" :key="i">
              <td class="mono">{{ p.name }}</td>
              <td class="mono">{{ p.value }}</td>
              <td class="text-dim mono">{{ p.source }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showPropsFor = null">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>
