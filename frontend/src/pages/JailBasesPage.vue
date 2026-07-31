<script setup>
import { ref, reactive, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';
import FilePicker from '../components/ui/FilePicker.vue';
import TaskConsole from '../components/ui/TaskConsole.vue';

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

// Edit base system modal
const showEdit = ref(false);
const editBase = ref(null);
const editName = ref('');
const allSnaps = ref([]);
const staleSnaps = ref([]);
const editSnapChecked = ref(new Set());

// File picker
const pickerTarget = ref(null);
const pickerConfig = ref({ mode: 'dir', accept: [] });

function openPicker(target, mode = 'dir', accept = []) {
  pickerTarget.value = target;
  pickerConfig.value = { mode, accept };
}
function onPickerSelect(path) {
  if (pickerTarget.value) form[pickerTarget.value] = path;
  pickerTarget.value = null;
}

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
    const res = await api.post('/api/jails/bases', result);
    if (res.task_id) {
      // Download method — show task modal with streaming output.
      showCreate.value = false;
      showTaskModal(res.task_id, result.name);
    } else {
      toast.toast(t('jails.baseCreated'));
      showCreate.value = false;
      await load();
    }
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// Task output modal (for download method)
const showTask = ref(false);
const taskDone = ref(false);
const taskName = ref('');
const activeTaskId = ref('');

function showTaskModal(taskId, name) {
  taskName.value = name;
  taskDone.value = false;
  activeTaskId.value = taskId;
  showTask.value = true;
}

async function onTaskDone({ success, output }) {
  taskDone.value = true;
  if (success) {
    toast.toast(t('jails.baseCreated'));
    await load();
  } else {
    await alert(t('common.operationFailed'), output.split('\n').filter(l => l).slice(-5).join('\n'));
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
async function openEdit(base) {
  editBase.value = base;
  editName.value = base.name;
  allSnaps.value = [];
  staleSnaps.value = [];
  editSnapChecked.value = new Set();
  showEdit.value = true;
  if (base.type === 'zfs') {
    const registered = new Set(base.snapshots || []);
    let live = [];
    try {
      live = await api.get(`/api/jails/bases/snapshots?name=${encodeURIComponent(base.source_path)}`);
    } catch (e) { live = []; }
    allSnaps.value = live;
    const liveSet = new Set(live);
    // Stale = registered snapshots that no longer exist on disk.
    staleSnaps.value = [...registered].filter(s => !liveSet.has(s));
    // Pre-check only the snapshots that still exist; stale ones are shown
    // separately (disabled) and dropped from the submitted list on save.
    editSnapChecked.value = new Set([...registered].filter(s => liveSet.has(s)));
  }
}

async function saveEdit() {
  const body = {};
  const newName = editName.value.trim();
  if (newName && newName !== editBase.value.name) body.name = newName;
  if (editBase.value.type === 'zfs') {
    const snaps = [...editSnapChecked.value];
    // Block accidental wipe only when live snapshots exist to choose from.
    // If every snapshot is gone (allSnaps empty), allow clearing so a rename
    // or stale-ref cleanup is never blocked.
    if (!snaps.length && allSnaps.value.length) {
      await alert(t('common.operationFailed'), t('jails.noSnapshotsSelected'));
      return;
    }
    body.snapshots = snaps;
  }
  if (!body.name && !body.snapshots) { showEdit.value = false; return; }
  try {
    await api.put(`/api/jails/bases/${encodeURIComponent(editBase.value.name)}`, body);
    toast.toast(t('common.saved'));
    showEdit.value = false;
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
              <button class="btn-secondary btn-sm" @click="openEdit(b)">{{ t('common.edit') }}</button>
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
          <div class="input-with-btn">
            <input type="text" v-model="form.txz_path" placeholder="/path/to/base.txz" />
            <button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('txz_path', 'file', ['.txz'])"><i class="fa-solid fa-folder-open"></i></button>
          </div>
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
            <div style="max-height:160px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);">
              <label v-for="s in snapshots" :key="s" class="checkbox-row" style="display:flex;">
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
            <div class="input-with-btn">
              <input type="text" v-model="form.import_sharedfs" placeholder="/usr/jails/sharedfs" />
              <button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('import_sharedfs')"><i class="fa-solid fa-folder-open"></i></button>
            </div>
          </div>
          <div class="field">
            <label>{{ t('jails.templateDir') }} <span style="color:var(--danger)">*</span></label>
            <div class="input-with-btn">
              <input type="text" v-model="form.import_template" placeholder="/usr/jails/template" />
              <button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('import_template')"><i class="fa-solid fa-folder-open"></i></button>
            </div>
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
            <div class="input-with-btn">
              <input type="text" v-model="form.new_sharedfs" :placeholder="`/usr/jails/sharedfs/${form.name || 'name'}`" />
              <button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('new_sharedfs')"><i class="fa-solid fa-folder-open"></i></button>
            </div>
          </div>
          <div class="field">
            <label>{{ t('jails.newTemplateDir') }} <span style="color:var(--danger)">*</span></label>
            <div class="input-with-btn">
              <input type="text" v-model="form.new_template" :placeholder="`/usr/jails/template/${form.name || 'name'}`" />
              <button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('new_template')"><i class="fa-solid fa-folder-open"></i></button>
            </div>
          </div>
        </template>

        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showCreate = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.confirm') }}</button>
        </div>
      </form>
    </div>
  </div>

  <!-- Edit base system modal -->
  <div v-if="showEdit" class="modal-overlay">
    <div class="modal" style="max-width:520px;">
      <h3>{{ t('common.edit') }} — {{ editBase?.name }}</h3>
      <form @submit.prevent="saveEdit">
        <div class="field">
          <label>{{ t('common.name') }} <span style="color:var(--danger)">*</span></label>
          <input type="text" v-model="editName" required />
        </div>
        <template v-if="editBase?.type === 'zfs'">
          <p class="text-dim" style="margin:0 0 8px;">{{ editBase?.source_path }}</p>
          <div class="field">
            <label>{{ t('jails.selectSnapshots') }} <span style="color:var(--danger)">*</span></label>
            <div style="max-height:200px;overflow-y:auto;border:1px solid var(--border);border-radius:var(--radius);">
              <span v-if="!allSnaps.length && !staleSnaps.length" class="text-dim" style="display:block;padding:6px 14px;">{{ t('jails.noSnapshots') }}</span>
              <label v-for="s in allSnaps" :key="s" class="checkbox-row" style="display:flex;">
                <input type="checkbox" :value="s" :checked="editSnapChecked.has(s)" @change="editSnapChecked.has(s) ? editSnapChecked.delete(s) : editSnapChecked.add(s)" />
                {{ s.includes('@') ? s.split('@').pop() : s }}
              </label>
              <label v-for="s in staleSnaps" :key="`stale-${s}`" class="checkbox-row" style="display:flex;opacity:0.5;">
                <input type="checkbox" disabled />
                <span style="text-decoration:line-through;">{{ s.includes('@') ? s.split('@').pop() : s }}</span>
                <span class="text-dim" style="margin-left:2px;">({{ t('common.deleted') }})</span>
              </label>
            </div>
          </div>
        </template>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showEdit = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.save') }}</button>
        </div>
      </form>
    </div>
  </div>

  <FilePicker
    v-if="pickerTarget"
    :mode="pickerConfig.mode"
    :accept="pickerConfig.accept"
    :initial-path="form[pickerTarget] || '/'"
    @select="onPickerSelect"
    @close="pickerTarget = null"
  />

  <!-- Task output modal (download method) -->
  <div v-if="showTask" class="modal-overlay">
    <div class="modal" style="max-width:680px;">
      <h3>
        <span v-if="!taskDone" class="spinner"></span>
        {{ t('jails.baseDownloading', { name: taskName }) }}
      </h3>
      <TaskConsole :task-id="activeTaskId" style="margin-bottom:12px;" @done="onTaskDone" />
      <div class="modal-actions">
        <button class="btn-secondary" :disabled="!taskDone" @click="showTask = false">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>
