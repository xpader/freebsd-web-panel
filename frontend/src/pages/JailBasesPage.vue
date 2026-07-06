<script setup>
import { ref, reactive, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const bases = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');

// Create modal
const showCreate = ref(false);
const datasets = ref([]);
const mirrors = ref([]);
const snapshots = ref([]);
const snapChecked = ref(new Set());
const form = reactive({
  name: '', method: '', type: '',
  // import+zfs
  import_dataset: '',
  // import+sharedfs
  import_sharedfs: '', import_template: '',
  // from-txz
  txz_path: '',
  // download
  mirror: '', version: '', download_url: '',
  // txz/download+zfs
  dataset: '', snapshot_name: '',
  // txz/download+sharedfs
  new_sharedfs: '', new_template: '',
});

// Edit snapshots modal
const showEditSnap = ref(false);
const editBase = ref(null);
const allSnaps = ref([]);
const editSnapChecked = ref(new Set());

async function load() {
  if (!bases.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    bases.value = await api.get('/api/jails/bases');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

function openCreate() {
  Object.assign(form, {
    name: '', method: '', type: '',
    import_dataset: '', import_sharedfs: '', import_template: '',
    txz_path: '', mirror: '', version: '', download_url: '',
    dataset: '', snapshot_name: '', new_sharedfs: '', new_template: '',
  });
  datasets.value = [];
  mirrors.value = [];
  snapshots.value = [];
  snapChecked.value = new Set();
  showCreate.value = true;

  // Load datasets for import dropdown
  api.get('/api/zfs/datasets').then((tree) => {
    datasets.value = flattenDatasets(tree);
  }).catch(() => {});

  // Load mirrors
  api.get('/api/jails/bases/mirrors').then((m) => {
    mirrors.value = m;
    if (m.length) form.mirror = m[0].url;
  }).catch(() => {
    mirrors.value = [{ name: 'Official', url: 'https://download.freebsd.org' }];
    form.mirror = 'https://download.freebsd.org';
  });

  // Pre-fill version
  api.get('/api/system/info').then((info) => {
    if (info.os_release) form.version = info.os_release;
  }).catch(() => {});
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

function updateDownloadUrl() {
  const version = form.version.trim();
  if (!form.mirror || !version) return;
  const arch = navigator.userAgent.includes('aarch64') ? 'arm64' : 'amd64';
  const branch = version.includes('RELEASE') ? 'releases' : 'snapshots';
  form.download_url = `${form.mirror}/${branch}/${arch}/${version}/base.txz`;
}

async function onDatasetChange() {
  if (!form.import_dataset) { snapshots.value = []; return; }
  try {
    snapshots.value = await api.get(`/api/jails/bases/snapshots?name=${encodeURIComponent(form.import_dataset)}`);
    snapChecked.value = new Set();
  } catch { snapshots.value = []; }
}

async function submitCreate() {
  const result = { name: form.name, method: form.method, type: form.type };
  if (form.method === 'import') {
    if (form.type === 'zfs') {
      const snaps = [...snapChecked.value];
      if (!form.import_dataset || !snaps.length) return;
      result.source_path = form.import_dataset;
      result.snapshots = snaps;
    } else {
      if (!form.import_sharedfs || !form.import_template) return;
      result.source_path = form.import_template;
      result.sharedfs_path = form.import_sharedfs;
    }
  } else if (form.method === 'from-txz') {
    result.txz_path = form.txz_path;
    if (!result.txz_path) return;
    if (form.type === 'zfs') {
      result.dataset = form.dataset;
      result.snapshot_name = form.snapshot_name || null;
      if (!result.dataset) return;
    } else {
      result.sharedfs_path = form.new_sharedfs;
      result.template_path = form.new_template;
      if (!result.sharedfs_path || !result.template_path) return;
    }
  } else if (form.method === 'download') {
    result.download_url = form.download_url;
    if (!result.download_url) return;
    if (form.type === 'zfs') {
      result.dataset = form.dataset;
      result.snapshot_name = form.snapshot_name || null;
      if (!result.dataset) return;
    } else {
      result.sharedfs_path = form.new_sharedfs;
      result.template_path = form.new_template;
      if (!result.sharedfs_path || !result.template_path) return;
    }
  }
  try {
    await api.post('/api/jails/bases', result);
    toast.toast(t('jails.baseCreated'));
    showCreate.value = false;
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function deleteBase(name) {
  if (!await confirm(t('common.delete'), t('jails.deleteBaseConfirm', { name }))) return;
  try {
    await api.del(`/api/jails/bases/${encodeURIComponent(name)}`);
    toast.toast(t('jails.baseDeleted'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function editSnapshots(base) {
  editBase.value = base;
  editSnapChecked.value = new Set(base.snapshots || []);
  allSnaps.value = [];
  showEditSnap.value = true;
  try {
    allSnaps.value = await api.get(`/api/jails/bases/snapshots?name=${encodeURIComponent(base.source_path)}`);
  } catch (e) {
    allSnaps.value = [];
  }
}

async function saveSnapshots() {
  const snaps = [...editSnapChecked.value];
  if (!snaps.length) {
    await alert(t('common.operationFailed'), t('jails.noSnapshotsSelected'));
    return;
  }
  try {
    await api.put(`/api/jails/bases/${encodeURIComponent(editBase.value.name)}`, { snapshots: snaps });
    toast.toast(t('jails.snapshotsUpdated'));
    showEditSnap.value = false;
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

function typeBadge(b) {
  return b.type === 'sharedfs'
    ? { cls: 'badge-warn', text: 'SharedFS' }
    : { cls: 'badge-success', text: 'ZFS' };
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('jails.basesTitle') }}</h1>
    <p>{{ t('jails.basesSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <span class="text-dim">{{ t('jails.basesCount', { n: bases.length }) }}</span>
    <div class="flex">
      <button @click="openCreate"><i class="fa-solid fa-plus"></i> {{ t('jails.createBase') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th><th>{{ t('jails.sourcePath') }}</th><th>{{ t('common.type') }}</th>
        <th>{{ t('jails.snapshots') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="5" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="5" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!bases.length"><td colspan="5" class="empty">{{ t('jails.noBases') }}</td></tr>
        <tr v-for="b in bases" :key="b.name">
          <td class="mono"><strong>{{ b.name }}</strong></td>
          <td class="mono text-dim">{{ b.source_path }}</td>
          <td><span :class="['badge', typeBadge(b).cls]">{{ typeBadge(b).text }}</span></td>
          <td class="mono text-dim">{{ b.snapshots?.length || '—' }}</td>
          <td>
            <div class="btn-group">
              <button v-if="b.type === 'zfs'" class="btn-secondary btn-sm" @click="editSnapshots(b)">{{ t('common.edit') }}</button>
              <button class="btn-secondary btn-sm" @click="deleteBase(b.name)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Create base modal -->
  <div v-if="showCreate" class="modal-overlay">
    <div class="modal" style="max-width:600px;">
      <h3>{{ t('jails.createBase') }}</h3>
      <form @submit.prevent="submitCreate">
        <div class="field">
          <label>{{ t('common.name') }} <span style="color:var(--danger)">*</span></label>
          <input type="text" v-model="form.name" required placeholder="freebsd-15.1" />
        </div>
        <div class="field">
          <label>{{ t('jails.creationMethod') }} <span style="color:var(--danger)">*</span></label>
          <select v-model="form.method" required>
            <option value="">{{ t('common.pleaseSelect') }}</option>
            <option value="import">{{ t('jails.methodImport') }}</option>
            <option value="from-txz">{{ t('jails.methodFromTxz') }}</option>
            <option value="download">{{ t('jails.methodDownload') }}</option>
          </select>
        </div>

        <!-- from-txz: txz path -->
        <div v-if="form.method === 'from-txz'" class="field">
          <label>{{ t('jails.baseTxzFile') }} <span style="color:var(--danger)">*</span></label>
          <input type="text" v-model="form.txz_path" placeholder="/path/to/base.txz" />
        </div>

        <!-- download: mirror + version + url -->
        <template v-if="form.method === 'download'">
          <div style="display:flex;gap:12px;">
            <div class="field" style="flex:1;">
              <label>{{ t('jails.mirror') }}</label>
              <select v-model="form.mirror" @change="updateDownloadUrl">
                <option v-for="m in mirrors" :key="m.url" :value="m.url">{{ m.name }}</option>
              </select>
            </div>
            <div class="field" style="flex:1;">
              <label>{{ t('jails.version') }}</label>
              <input type="text" v-model="form.version" @input="updateDownloadUrl" placeholder="" />
            </div>
          </div>
          <div class="field">
            <label>{{ t('jails.downloadUrl') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="form.download_url" :placeholder="t('jails.downloadUrlPh')" />
          </div>
        </template>

        <div class="field">
          <label>{{ t('common.type') }} <span style="color:var(--danger)">*</span></label>
          <select v-model="form.type" required>
            <option value="">{{ t('common.pleaseSelect') }}</option>
            <option value="zfs">ZFS {{ t('jails.dataset') }}</option>
            <option value="sharedfs">SharedFS</option>
          </select>
        </div>

        <!-- import + ZFS -->
        <template v-if="form.method === 'import' && form.type === 'zfs'">
          <div class="field">
            <label>{{ t('jails.zfsDataset') }} <span style="color:var(--danger)">*</span></label>
            <select v-model="form.import_dataset" @change="onDatasetChange">
              <option value="">{{ t('common.pleaseSelect') }}</option>
              <option v-for="d in datasets" :key="d" :value="d">{{ d }}</option>
            </select>
          </div>
          <div v-if="snapshots.length" class="field">
            <label>{{ t('jails.selectSnapshots') }} <span style="color:var(--danger)">*</span></label>
            <div style="max-height:160px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);padding:8px;">
              <label v-for="s in snapshots" :key="s" style="display:flex;align-items:center;gap:6px;padding:3px 0;font-size:13px;cursor:pointer;">
                <input type="checkbox" :value="s" :checked="snapChecked.has(s)" @change="snapChecked.has(s) ? snapChecked.delete(s) : snapChecked.add(s)" />
                {{ s.includes('@') ? s.split('@').pop() : s }}
              </label>
            </div>
          </div>
        </template>

        <!-- import + SharedFS -->
        <template v-if="form.method === 'import' && form.type === 'sharedfs'">
          <div class="field">
            <label>{{ t('jails.sharedfsDir') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="form.import_sharedfs" placeholder="/usr/jails/sharedfs" />
          </div>
          <div class="field">
            <label>{{ t('jails.templateDir') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="form.import_template" placeholder="/usr/jails/template" />
          </div>
        </template>

        <!-- from-txz / download + ZFS -->
        <template v-if="(form.method === 'from-txz' || form.method === 'download') && form.type === 'zfs'">
          <div class="field">
            <label>{{ t('jails.newDataset') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="form.dataset" :placeholder="`zroot/jails/bases/${form.name || 'name'}`" />
          </div>
          <div class="field">
            <label>{{ t('jails.snapshotName') }}</label>
            <input type="text" v-model="form.snapshot_name" :placeholder="t('jails.snapshotNamePh')" />
          </div>
        </template>

        <!-- from-txz / download + SharedFS -->
        <template v-if="(form.method === 'from-txz' || form.method === 'download') && form.type === 'sharedfs'">
          <div class="field">
            <label>{{ t('jails.newSharedfsDir') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="form.new_sharedfs" :placeholder="`/usr/jails/sharedfs/${form.name || 'name'}`" />
          </div>
          <div class="field">
            <label>{{ t('jails.newTemplateDir') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="form.new_template" :placeholder="`/usr/jails/template/${form.name || 'name'}`" />
          </div>
        </template>

        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showCreate = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.confirm') }}</button>
        </div>
      </form>
    </div>
  </div>

  <!-- Edit snapshots modal -->
  <div v-if="showEditSnap" class="modal-overlay">
    <div class="modal" style="max-width:520px;">
      <h3>{{ t('jails.editSnapshots') }} — {{ editBase?.name }}</h3>
      <p class="text-dim" style="margin-bottom:12px;">{{ editBase?.source_path }}</p>
      <div class="field">
        <label>{{ t('jails.selectSnapshots') }} <span style="color:var(--danger)">*</span></label>
        <div style="max-height:200px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);padding:8px;">
          <span v-if="!allSnaps.length" class="text-dim">{{ t('jails.noSnapshots') }}</span>
          <label v-for="s in allSnaps" :key="s" style="display:flex;align-items:center;gap:6px;padding:3px 0;font-size:13px;cursor:pointer;">
            <input type="checkbox" :value="s" :checked="editSnapChecked.has(s)" @change="editSnapChecked.has(s) ? editSnapChecked.delete(s) : editSnapChecked.add(s)" />
            {{ s.includes('@') ? s.split('@').pop() : s }}
          </label>
        </div>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showEditSnap = false">{{ t('common.cancel') }}</button>
        <button @click="saveSnapshots">{{ t('common.save') }}</button>
      </div>
    </div>
  </div>
</template>
