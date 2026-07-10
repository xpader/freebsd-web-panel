<script setup>
import { ref, reactive, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';
import FilePicker from '../components/ui/FilePicker.vue';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const datastores = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');

// Create modal
const showCreate = ref(false);
const datasets = ref([]);
const form = reactive({ name: '', type: '', dataset: '', dirPath: '' });

// File picker
const showPicker = ref(false);

function onPickerSelect(path) {
  form.dirPath = path;
  showPicker.value = false;
}

function flattenDatasets(tree) {
  const result = [];
  function walk(nodes) {
    for (const n of nodes) {
      result.push(n.name);
      if (n.children) walk(n.children);
    }
  }
  walk(tree);
  return result;
}

function openCreate() {
  Object.assign(form, { name: '', type: '', dataset: '', dirPath: '' });
  datasets.value = [];
  showCreate.value = true;
  api.get('/api/zfs/datasets').then((tree) => {
    datasets.value = flattenDatasets(tree);
  }).catch(() => {});
}

async function submitCreate() {
  const spec = form.type === 'zfs' ? `zfs:${form.dataset}` : form.dirPath.trim();
  try {
    await api.post('/api/bhyve/datastores', { name: form.name, spec });
    toast.toast(t('bhyve.dsCreated', { name: form.name }));
    showCreate.value = false;
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

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
      <button @click="openCreate"><i class="fa-solid fa-plus"></i> {{ t('common.create') }}</button>
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

  <!-- Create datastore modal -->
  <div v-if="showCreate" class="modal-overlay">
    <div class="modal" style="max-width:560px;">
      <h3>{{ t('bhyve.dsCreateTitle') }}</h3>
      <form @submit.prevent="submitCreate">
        <div class="field">
          <label>{{ t('common.name') }} <span style="color:var(--danger)">*</span></label>
          <input type="text" v-model="form.name" required :placeholder="t('bhyve.dsNamePlaceholder')" />
        </div>
        <div class="field">
          <label>{{ t('common.type') }} <span style="color:var(--danger)">*</span></label>
          <select v-model="form.type" required>
            <option value="">{{ t('common.pleaseSelect') }}</option>
            <option value="zfs">ZFS</option>
            <option value="directory">{{ t('bhyve.dsTypeDirectory') }}</option>
          </select>
        </div>

        <!-- ZFS dataset selector -->
        <div v-if="form.type === 'zfs'" class="field">
          <label>{{ t('bhyve.dsZfsDataset') }} <span style="color:var(--danger)">*</span></label>
          <select v-model="form.dataset" required>
            <option value="">{{ t('common.pleaseSelect') }}</option>
            <option v-for="d in datasets" :key="d" :value="d">{{ d }}</option>
          </select>
        </div>

        <!-- Directory path input -->
        <div v-if="form.type === 'directory'" class="field">
          <label>{{ t('bhyve.dsDirPath') }} <span style="color:var(--danger)">*</span></label>
          <div class="input-with-btn">
            <input type="text" v-model="form.dirPath" required placeholder="/home/vm-data" />
            <button type="button" class="btn-secondary btn-sm fp-trigger" @click="showPicker = true"><i class="fa-solid fa-folder-open"></i></button>
          </div>
        </div>

        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showCreate = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.create') }}</button>
        </div>
      </form>
    </div>
  </div>

  <FilePicker
    v-if="showPicker"
    mode="dir"
    :initial-path="form.dirPath || '/'"
    @select="onPickerSelect"
    @close="showPicker = false"
  />
</template>
