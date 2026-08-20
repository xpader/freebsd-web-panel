<script setup>
import { ref, computed, onMounted } from 'vue';
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
const editingProp = ref(null);
const editValue = ref('');
const savingProp = ref(false);
const propSchema = ref({ enums: {}, booleans: [], readonly: [] });
const propFilter = ref('');

const filteredProps = computed(() => {
  if (!propsData.value) return [];
  const q = propFilter.value.trim().toLowerCase();
  if (!q) return propsData.value;
  return propsData.value.filter((p) => p.name.toLowerCase().includes(q));
});

function isEditable(prop) {
  return !propSchema.value.readonly.includes(prop);
}

function propInputType(prop) {
  if (propSchema.value.enums[prop]) return 'select';
  if (propSchema.value.booleans.includes(prop)) return 'bool';
  return 'text';
}

function startEdit(prop, currentVal) {
  editingProp.value = prop;
  editValue.value = currentVal;
}

function cancelEdit() {
  editingProp.value = null;
  editValue.value = '';
}

async function saveProp(prop) {
  savingProp.value = true;
  try {
    await api.put(`/api/zfs/dataset/properties?name=${encodeURIComponent(showPropsFor.value)}`, {
      properties: { [prop]: editValue.value },
    });
    toast.toast(t('common.saved'));
    cancelEdit();
    await refreshProps();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: e.message || '' }), e.message || t('common.operationFailed'));
  } finally {
    savingProp.value = false;
  }
}

async function resetProp(prop) {
  if (!await confirm(t('zfs.dsPropResetTitle'), t('zfs.dsPropResetConfirm', { name: prop }))) return;
  try {
    await api.post(`/api/zfs/dataset/inherit?name=${encodeURIComponent(showPropsFor.value)}&property=${encodeURIComponent(prop)}`);
    toast.toast(t('zfs.dsPropResetDone'));
    await refreshProps();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

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
  const fields = [
    { key: 'kind', label: t('common.type'), type: 'radio', value: 'filesystem', options: [
      { value: 'filesystem', label: t('zfs.dsTypeFilesystem') },
      { value: 'volume', label: t('zfs.dsTypeVolume') },
    ] },
    { key: 'name', label: t('zfs.dsNameLabel'), placeholder: t('zfs.dsNamePlaceholder'), required: true },
    { key: 'volsize', label: t('zfs.dsVolsize'), placeholder: t('zfs.dsSizePlaceholder'), showIf: { kind: 'volume' }, requiredIf: { kind: 'volume' }, tooltip: t('zfs.dsVolsizeTip') },
    { key: 'mountpoint', label: t('zfs.mountpoint'), placeholder: t('zfs.dsMountpointPlaceholder'), showIf: { kind: 'filesystem' } },
    { key: 'compression', label: t('zfs.compression'), type: 'select', half: true, tooltip: t('zfs.dsCompressionTip'), options: [{ value: '', label: t('common.default') }, ...propSchema.value.enums.compression?.map(v => ({ value: v, label: v })) || []] },
    { key: 'quota', label: 'Quota', half: true, placeholder: t('zfs.dsSizePlaceholder'), tooltip: t('zfs.dsQuotaTip') },
    { key: 'reservation', label: 'Reservation', half: true, placeholder: t('zfs.dsSizePlaceholder'), tooltip: t('zfs.dsReservationTip') },
    { key: 'recordsize', label: t('zfs.dsRecordsize'), type: 'select', half: true, showIf: { kind: 'filesystem' }, tooltip: t('zfs.dsRecordsizeTip'), options: [{ value: '', label: t('common.default') }, ...['512','1K','2K','4K','8K','16K','32K','64K','128K'].map(v => ({ value: v, label: v }))] },
    { key: 'dedup', label: 'Dedup', type: 'select', half: true, tooltip: t('zfs.dsDedupTip'), options: [{ value: '', label: t('common.default') }, ...((propSchema.value.enums.dedup || ['on','off','verify']).map(v => ({ value: v, label: v }))) ] },
    { key: 'atime', label: t('zfs.dsAtime'), type: 'select', half: true, showIf: { kind: 'filesystem' }, tooltip: t('zfs.dsAtimeTip'), options: [{ value: '', label: t('common.default') }, { value: 'on', label: 'on' }, { value: 'off', label: 'off' }] },
  ];
  await formModal(t('zfs.dsCreateTitle'), fields, {
    submitHandler: async (result) => {
      const isVolume = result.kind === 'volume';
      const props = {};
      for (const [k, v] of Object.entries(result)) {
        if (k === 'name' || k === 'kind' || !v || !v.trim()) continue;
        props[k] = v.trim();
      }
      if (isVolume) {
        delete props.mountpoint;
        delete props.canmount;
      } else {
        delete props.volsize;
      }
      const body = { name: result.name };
      if (isVolume) body.kind = 'volume';
      if (Object.keys(props).length) body.properties = props;
      await api.post('/api/zfs/datasets', body);
      toast.toast(t('zfs.dsCreated'));
      await load();
    },
  });
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

async function promoteDataset(ds) {
  if (!await confirm(t('zfs.promoteTitle'), t('zfs.promoteConfirm', { name: ds.name, origin: ds.origin }))) return;
  try {
    await api.post(`/api/zfs/dataset/promote?name=${encodeURIComponent(ds.name)}`);
    toast.toast(t('zfs.promoteDone', { name: ds.name }));
    await load();
  } catch (e) {
    await alert(t('zfs.promoteFailed'), e.message || t('zfs.promoteFailed'));
  }
}

async function refreshProps() {
  try {
    propsData.value = await api.get(`/api/zfs/dataset/properties?name=${encodeURIComponent(showPropsFor.value)}`);
  } catch (e) {
    await alert(t('zfs.dsPropsFailed'), e.message || t('zfs.dsPropsFailed'));
  }
}

async function showProps(name) {
  try {
    propsData.value = await api.get(`/api/zfs/dataset/properties?name=${encodeURIComponent(name)}`);
    showPropsFor.value = name;
    editingProp.value = null;
    propFilter.value = '';
  } catch (e) {
    await alert(t('zfs.dsPropsFailed'), e.message || t('zfs.dsPropsFailed'));
  }
}

onMounted(async () => {
  try {
    propSchema.value = await api.get('/api/zfs/dataset/prop-schema');
  } catch (_) { /* schema optional, fallback to all-text inputs */ }
  load();
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('zfs.dsTitle') }}</h1>
    <p>{{ t('zfs.dsSubtitle') }}</p>
    <div class="flex" style="margin-left:auto;">
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
              <button v-if="ds.origin" class="btn-secondary btn-sm" @click="promoteDataset(ds)">{{ t('zfs.promote') }}</button>
              <button v-if="ds.name.includes('/')" class="btn-danger btn-sm" @click="deleteDataset(ds.name)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Properties modal -->
  <div v-if="showPropsFor" class="modal-overlay">
    <div class="modal" style="max-width:700px;">
      <h3>{{ t('zfs.propsTitle', { name: showPropsFor }) }}</h3>
      <input v-model="propFilter" :placeholder="t('zfs.dsPropFilter')" style="width:100%;margin-bottom:8px;font-size:13px;">
      <div style="max-height:400px;overflow-y:auto;">
        <table style="font-size:12px;">
          <thead><tr><th>{{ t('common.name') }}</th><th style="min-width:200px;">{{ t('common.value') }}</th><th>{{ t('zfs.source') }}</th></tr></thead>
          <tbody>
            <tr v-for="(p, i) in filteredProps" :key="i">
              <td class="mono">{{ p.name }}</td>
              <td>
                <!-- Editing mode -->
                <div v-if="editingProp === p.name" class="inline-edit">
                  <select v-if="propInputType(p.name) === 'select'" v-model="editValue">
                    <option v-for="opt in propSchema.enums[p.name]" :key="opt" :value="opt">{{ opt }}</option>
                  </select>
                  <select v-else-if="propInputType(p.name) === 'bool'" v-model="editValue">
                    <option value="on">on</option>
                    <option value="off">off</option>
                  </select>
                  <input v-else v-model="editValue" class="mono" @keyup.enter="saveProp(p.name)" />
                  <button class="inline-edit-btn" @click="saveProp(p.name)" :disabled="savingProp"><i class="fa-solid fa-check"></i></button>
                  <button class="inline-edit-btn" @click="cancelEdit"><i class="fa-solid fa-xmark"></i></button>
                </div>
                <!-- Display mode -->
                <div v-else class="flex" style="gap:4px;align-items:center;">
                  <span class="mono">{{ p.value }}</span>
                  <div v-if="isEditable(p.name)" class="btn-group">
                    <button class="btn-secondary btn-sm" style="padding:1px 6px;" @click="startEdit(p.name, p.value)"><i class="fa-solid fa-pen"></i></button>
                    <button v-if="p.source === 'local'" class="btn-secondary btn-sm" style="padding:1px 6px;" @click="resetProp(p.name)" :title="t('zfs.dsPropResetTitle')"><i class="fa-solid fa-rotate-left"></i></button>
                  </div>
                </div>
              </td>
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

<style scoped>
.inline-edit {
  display: flex;
  align-items: stretch;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  width: fit-content;
}
.inline-edit input,
.inline-edit select {
  width: 130px;
  padding: 4px 8px;
  font-size: 12px;
  border: none;
  border-radius: 0;
  background: var(--bg);
}
.inline-edit select { width: auto; min-width: 80px; }
.inline-edit-btn {
  flex-shrink: 0;
  padding: 4px 8px;
  font-size: 12px;
  border: none;
  border-left: 1px solid var(--border);
  border-radius: 0;
  background: var(--bg-elev2);
  color: var(--text);
  cursor: pointer;
}
.inline-edit-btn:hover { background: var(--border); }
.inline-edit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
