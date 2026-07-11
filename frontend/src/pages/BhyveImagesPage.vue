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

const images = ref([]);
const vms = ref([]);
const datastores = ref([]);
const loading = ref(true);
const error = ref('');

async function load() {
  loading.value = true;
  error.value = '';
  try {
    const [imgList, vmList, dsList] = await Promise.all([
      api.get('/api/bhyve/images'),
      api.get('/api/bhyve/vms'),
      api.get('/api/bhyve/datastores'),
    ]);
    images.value = imgList;
    vms.value = vmList;
    datastores.value = dsList;
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function createImage() {
  if (!vms.value.length) {
    await alert(t('bhyve.imgCreateFailed'), t('bhyve.noVms'));
    return;
  }
  const vmOptions = vms.value.map(v => ({ value: v.name, label: v.name }));
  const result = await formModal(t('bhyve.imgCreateTitle'), [
    { key: 'name', label: t('bhyve.imgSourceVm'), type: 'select', options: vmOptions, required: true },
    { key: 'description', label: t('common.description'), placeholder: t('common.description') },
  ], t('common.create'));
  if (!result) return;
  try {
    const res = await api.post('/api/bhyve/images', {
      name: result.name,
      description: result.description || undefined,
    });
    toast.toast(t('bhyve.imgCreated', { uuid: res.uuid }));
    await load();
  } catch (e) {
    await alert(t('bhyve.imgCreateFailed'), e.message || t('bhyve.imgCreateFailed'));
  }
}

async function provisionImage(img) {
  const dsOptions = datastores.value.map(d => ({ value: d.name, label: d.name }));
  const result = await formModal(t('bhyve.imgProvisionTitle'), [
    { key: 'new_name', label: t('bhyve.imgNewName'), placeholder: 'my-new-vm', required: true },
    { key: 'datastore', label: t('bhyve.datastore'), type: 'select', options: dsOptions },
  ], t('common.create'));
  if (!result) return;
  try {
    await api.post(`/api/bhyve/images/${encodeURIComponent(img.uuid)}/provision`, {
      new_name: result.new_name,
      datastore: result.datastore || undefined,
    });
    toast.toast(t('bhyve.imgProvisioned', { name: result.new_name }));
    await load();
  } catch (e) {
    await alert(t('bhyve.imgProvisionFailed'), e.message || t('bhyve.imgProvisionFailed'));
  }
}

async function destroyImage(img) {
  if (!await confirm(t('bhyve.imgDeleteTitle'), t('bhyve.imgDeleteConfirm', { uuid: img.uuid }))) return;
  try {
    await api.del(`/api/bhyve/images/${encodeURIComponent(img.uuid)}`);
    toast.toast(t('bhyve.imgDeleted'));
    await load();
  } catch (e) {
    await alert(t('bhyve.imgDeleteFailed'), e.message || t('bhyve.imgDeleteFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.tabImages') }}</h1>
    <p>{{ t('bhyve.imageSubtitle') }}</p>
  </div>

  <div class="toolbar">
    <span class="text-dim">{{ t('bhyve.imageCount', { n: images.length }) }}</span>
    <div class="flex">
      <button @click="createImage"><i class="fa-solid fa-plus"></i> {{ t('bhyve.imgCreateTitle') }}</button>
      <button @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>UUID</th>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('common.createdAt') }}</th>
        <th>{{ t('common.description') }}</th>
        <th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="5" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="5" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!images.length"><td colspan="5" class="empty">{{ t('bhyve.noImages') }}</td></tr>
        <tr v-for="img in images" :key="img.uuid">
          <td class="mono">{{ img.uuid }}</td>
          <td class="mono"><strong>{{ img.name }}</strong></td>
          <td class="mono">{{ img.created }}</td>
          <td>{{ img.description || '—' }}</td>
          <td>
            <div class="btn-group">
              <button class="btn-sm" @click="provisionImage(img)" :title="t('bhyve.imgProvisionTitle')"><i class="fa-solid fa-clone"></i></button>
              <button class="btn-sm btn-danger" @click="destroyImage(img)" :title="t('common.delete')"><i class="fa-solid fa-trash"></i></button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
