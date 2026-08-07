<script setup>
import { ref, reactive, computed, onMounted, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { api, authFetch } from '../lib/api.js';
import { fmtBytes, fmtDate, octStr } from '../lib/format.js';
import { useToast, useConfirm, useAlert } from '../composables/useDialog.js';
import { ROOT, fileIcon, joinPath, createTreeState } from '../lib/fileTree.js';
import FileTreeRow from '../components/ui/FileTreeRow.vue';
import PermissionDialog from '../components/ui/PermissionDialog.vue';

const { t } = useI18n();
const toast = useToast();
const confirm = useConfirm();
const alert = useAlert();

const ROOT_SLASH = ROOT;
const currentDir = ref('/root');
const viewMode = ref(localStorage.getItem('fwp_fm_view') || 'list');

const { expanded, treeChildren, toggleExpand, ensureAncestors, getChildren, invalidate, refreshAll } =
  createTreeState({ filterFn: (e) => e.is_dir });

const entries = ref([]);
const loading = ref(true);
const refreshing = ref(false);

// Modals
const showMkdir = ref(false);
const mkdirName = ref('');
const showRename = ref(false);
const renamePath = ref('');
const renameName = ref('');
const statInfo = ref(null);
const showChmod = ref(false);
const chmodInfo = ref(null);
const chmodMode = ref('0644');
const showChown = ref(false);
const chownInfo = ref(null);
const chownAccounts = ref(null);
const chownUid = ref('');
const chownGid = ref('');

async function openDir(path) {
  currentDir.value = path;
  if (path !== ROOT) await ensureAncestors(path);
  await loadListing(path);
}

async function loadListing(path) {
  if (!entries.value.length) loading.value = true;
  try {
    entries.value = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
  } catch (err) {
    entries.value = [];
  } finally {
    loading.value = false;
  }
}

async function refresh() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    await Promise.all([loadListing(currentDir.value), refreshTree()]);
  } finally {
    refreshing.value = false;
  }
}

function breadcrumbParts() {
  return currentDir.value.split('/').filter(Boolean);
}

function breadcrumbPath(idx) {
  return '/' + breadcrumbParts().slice(0, idx + 1).join('/');
}

async function refreshTree() {
  const parts = currentDir.value.split('/').filter(Boolean);
  const paths = [ROOT];
  let cur = '';
  for (const p of parts) { cur = cur + '/' + p; paths.push(cur); }
  await refreshAll(paths);
}

// Upload manager
const showUpload = ref(false);
const uploadQueue = reactive([]); // { id, file, name, size, progress, status, error }
const uploadInput = ref(null);
const dropzoneDragover = ref(false);
let uploadSeq = 0;

const activeUploadCount = computed(() => uploadQueue.filter((u) => u.status === 'uploading').length);

function openUploadModal() {
  showUpload.value = true;
}

function addFiles(files) {
  for (const file of files) {
    uploadQueue.push({
      id: ++uploadSeq,
      file,
      name: file.name,
      size: file.size,
      progress: 0,
      status: 'pending',
      error: null,
    });
  }
}

function onUploadPicked(ev) {
  addFiles([...ev.target.files]);
  ev.target.value = '';
}

function onDrop(ev) {
  ev.preventDefault();
  dropzoneDragover.value = false;
  if (ev.dataTransfer?.files?.length) addFiles([...ev.dataTransfer.files]);
}

function onDragover(ev) {
  ev.preventDefault();
  dropzoneDragover.value = true;
}

function onDragleave() {
  dropzoneDragover.value = false;
}

function removeUploadItem(id) {
  const idx = uploadQueue.findIndex((u) => u.id === id);
  if (idx >= 0) uploadQueue.splice(idx, 1);
}

function clearCompleted() {
  for (let i = uploadQueue.length - 1; i >= 0; i--) {
    if (uploadQueue[i].status === 'done' || uploadQueue[i].status === 'error') {
      uploadQueue.splice(i, 1);
    }
  }
}

function startUpload() {
  const pending = uploadQueue.filter((u) => u.status === 'pending');
  if (!pending.length) return;
  const dir = currentDir.value;
  for (const item of pending) {
    item.status = 'uploading';
    const url = `/api/files/upload?path=${encodeURIComponent(dir)}&filename=${encodeURIComponent(item.name)}`;
    const xhr = new XMLHttpRequest();
    xhr.upload.addEventListener('progress', (ev) => {
      if (ev.lengthComputable) {
        item.progress = Math.round((ev.loaded / ev.total) * 100);
      }
    });
    xhr.addEventListener('load', () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        item.status = 'done';
        item.progress = 100;
        toast.toast(t('fm.uploaded', { name: item.name }));
      } else {
        item.status = 'error';
        item.error = `HTTP ${xhr.status}`;
      }
      checkAllDone();
    });
    xhr.addEventListener('error', () => {
      item.status = 'error';
      item.error = 'Network error';
      checkAllDone();
    });
    xhr.addEventListener('abort', () => {
      item.status = 'error';
      item.error = 'Aborted';
      checkAllDone();
    });
    xhr.open('POST', url);
    const token = sessionStorage.getItem('fwp_token');
    if (token) xhr.setRequestHeader('Authorization', `Bearer ${token}`);
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.send(item.file);
  }
}

let allDoneToastShown = false;
function checkAllDone() {
  if (!uploadQueue.some((u) => u.status === 'uploading' || u.status === 'pending')) {
    const hasError = uploadQueue.some((u) => u.status === 'error');
    if (hasError) {
      alert(t('fm.uploadFailed', { name: '', msg: '' }), t('fm.uploadFailed', { name: '', msg: '' }));
    }
    loadListing(currentDir.value);
    invalidate(currentDir.value);
    refreshTree();
  }
}

async function downloadFile(path) {
  try {
    const res = await authFetch(`/api/files/download?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw { message: `Download failed (${res.status})` };
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = path.split('/').filter(Boolean).pop() || 'download';
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    toast.toast(t('fm.downloadStarted'));
  } catch (err) {
    await alert(t('fm.downloadFailed', { msg: '' }), t('fm.downloadFailed', { msg: err.message || '' }));
  }
}

function openMkdir() { mkdirName.value = ''; showMkdir.value = true; }

async function doMkdir() {
  const target = joinPath(currentDir.value, mkdirName.value);
  try {
    await api.post(`/api/files/mkdir?path=${encodeURIComponent(target)}`);
    toast.toast(t('fm.mkdirDone'));
    showMkdir.value = false;
    invalidate(currentDir.value);
    await refreshTree();
    await loadListing(currentDir.value);
  } catch (err) {
    await alert(t('fm.mkdirFailed', { msg: '' }), t('fm.mkdirFailed', { msg: err.message || '' }));
  }
}

function openRename(path) {
  renamePath.value = path;
  renameName.value = path.split('/').filter(Boolean).pop() || '';
  showRename.value = true;
}

async function doRename() {
  const parent = renamePath.value.split('/').filter(Boolean).slice(0, -1).join('/') || '/';
  const target = joinPath(parent, renameName.value);
  try {
    await api.post(`/api/files/rename?from=${encodeURIComponent(renamePath.value)}&to=${encodeURIComponent(target)}`);
    toast.toast(t('fm.renameDone'));
    showRename.value = false;
    invalidate(parent);
    await refreshTree();
    await loadListing(currentDir.value);
  } catch (err) {
    await alert(t('fm.renameFailed', { msg: '' }), t('fm.renameFailed', { msg: err.message || '' }));
  }
}

async function onDelete(path, isDir) {
  const name = path.split('/').filter(Boolean).pop() || path;
  const ok = await confirm(t('common.delete'),
    isDir ? t('fm.deleteConfirmDir', { name }) : t('fm.deleteConfirmFile', { name }));
  if (!ok) return;
  try {
    await api.del(`/api/files?path=${encodeURIComponent(path)}`);
    toast.toast(t('fm.deleteDone'));
    invalidate(path);
    await refreshTree();
    await loadListing(currentDir.value);
  } catch (err) {
    await alert(t('fm.deleteFailed', { msg: '' }), t('fm.deleteFailed', { msg: err.message || '' }));
  }
}

async function onStat(path) {
  try {
    statInfo.value = await api.get(`/api/files/stat?path=${encodeURIComponent(path)}`);
  } catch (err) {
    await alert(t('fm.statReadFailed', { msg: '' }), t('fm.statReadFailed', { msg: err.message || '' }));
  }
}

function openChmod(info) {
  statInfo.value = null;
  chmodInfo.value = info;
  chmodMode.value = octStr(info.mode & 0o7777);
  showChmod.value = true;
}

async function doChmod(mode) {
  try {
    await api.put(`/api/files/chmod?path=${encodeURIComponent(chmodInfo.value.path)}`, { mode: parseInt(mode, 8) });
    toast.toast(t('fm.permSaved'));
    showChmod.value = false;
    await loadListing(currentDir.value);
  } catch (err) {
    await alert(t('common.saveFailed', { msg: '' }), t('fm.saveFailed', { msg: err.message || '' }));
  }
}

async function openChown(info) {
  statInfo.value = null;
  chownInfo.value = info;
  chownUid.value = String(info.uid);
  chownGid.value = String(info.gid);
  showChown.value = true;
  if (!chownAccounts.value) {
    try { chownAccounts.value = await api.get('/api/files/accounts'); } catch {}
  }
}

async function doChown() {
  const body = {};
  if (chownUid.value) body.uid = parseInt(chownUid.value, 10);
  if (chownGid.value) body.gid = parseInt(chownGid.value, 10);
  try {
    await api.put(`/api/files/chown?path=${encodeURIComponent(chownInfo.value.path)}`, body);
    toast.toast(t('fm.ownerSaved'));
    showChown.value = false;
    await loadListing(currentDir.value);
  } catch (err) {
    await alert(t('common.saveFailed', { msg: '' }), t('fm.saveFailed', { msg: err.message || '' }));
  }
}

function setView(mode) {
  viewMode.value = mode;
  localStorage.setItem('fwp_fm_view', mode);
}


onMounted(async () => {
  expanded.add(ROOT);
  if (!treeChildren.has(ROOT)) {
    try {
      const list = await api.get(`/api/files/list?path=${encodeURIComponent(ROOT)}`);
      treeChildren.set(ROOT, list.filter((e) => e.is_dir));
    } catch { treeChildren.set(ROOT, []); }
  }
  await ensureAncestors(currentDir.value);
  await loadListing(currentDir.value);
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('fm.title') }}</h1>
    <p>{{ t('fm.subtitle') }}</p>
  </div>
  <div class="fm-wrap">
    <!-- Tree -->
    <div class="fm-tree">
      <div class="fm-tree-head">{{ t('fm.treeHead') }}</div>
      <div class="fm-tree-body">
        <!-- Root node -->
        <div class="fm-tree-node">
          <div :class="['fm-tree-row', { active: currentDir === ROOT }]" @click="openDir(ROOT)" style="padding-left:6px">
            <span class="fm-tree-arrow" @click.stop="toggleExpand(ROOT)">
              <i :class="expanded.has(ROOT) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
            </span>
            <span class="fm-tree-name"><span class="fm-tree-ico"><i class="fa-solid fa-folder-tree"></i></span>/</span>
          </div>
          <div v-if="expanded.has(ROOT)" class="fm-tree-children">
            <FileTreeRow
              v-for="d in (getChildren(ROOT) || [])"
              :key="d.path"
              :entry="d"
              :depth="1"
              :expanded="expanded"
              :tree-children="treeChildren"
              :toggle-expand="toggleExpand"
              :selected-path="currentDir"
              @rowclick="(e) => openDir(e.path)"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- Main -->
    <div class="fm-main">
      <div v-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

      <template v-else>
        <div class="fm-toolbar">
          <div class="fm-breadcrumb">
            <a class="fm-crumb" @click="openDir(ROOT)">/</a>
            <template v-for="(part, i) in breadcrumbParts()" :key="i">
              <span v-if="i > 0" class="fm-sep">/</span>
              <a class="fm-crumb" @click="openDir(breadcrumbPath(i))">{{ part }}</a>
            </template>
          </div>
          <div class="fm-actions">
            <button class="btn-secondary btn-sm" @click="refresh"><i class="fa-solid fa-rotate-right" :class="{ 'fa-spin': refreshing }"></i> {{ t('common.refresh') }}</button>
            <button class="btn-secondary btn-sm" @click="openUploadModal"><i class="fa-solid fa-upload"></i> {{ t('fm.upload') }}<span v-if="activeUploadCount" class="upload-badge">{{ activeUploadCount }}</span></button>
            <button class="btn-secondary btn-sm" @click="openMkdir"><i class="fa-solid fa-folder-plus"></i> {{ t('fm.mkdir') }}</button>
            <div class="fm-view-toggle">
              <button :class="['btn-secondary', 'btn-sm', { 'active-range': viewMode === 'list' }]" @click="setView('list')"><i class="fa-solid fa-list"></i> {{ t('fm.listView') }}</button>
              <button :class="['btn-secondary', 'btn-sm', { 'active-range': viewMode === 'grid' }]" @click="setView('grid')"><i class="fa-solid fa-table-cells"></i> {{ t('fm.gridView') }}</button>
            </div>
          </div>
        </div>

        <!-- List view -->
        <div v-if="viewMode === 'list'" class="fm-listing">
          <div v-if="!entries.length" class="empty">{{ t('fm.emptyDir') }}</div>
          <table v-else class="fm-table">
            <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('common.size') }}</th><th>{{ t('common.owner') }}</th><th>{{ t('common.group') }}</th><th>{{ t('common.permissions') }}</th><th>{{ t('fm.modified') }}</th><th>{{ t('common.actions') }}</th></tr></thead>
            <tbody>
              <tr v-for="e in entries" :key="e.path">
                <td class="fm-name-cell">
                  <div class="fm-cell-flex">
                    <span class="fm-row-ico"><i :class="fileIcon(e)"></i></span>
                    <a v-if="e.is_dir" class="fm-name-link" @click="openDir(e.path)">{{ e.name }}</a>
                    <span v-else class="fm-name">{{ e.name }}</span>
                  </div>
                </td>
                <td class="mono">{{ e.is_dir ? '—' : fmtBytes(e.size) }}</td>
                <td class="text-dim mono">{{ e.user }}</td>
                <td class="text-dim mono">{{ e.group }}</td>
                <td :class="['mono', (!e.is_dir && (e.mode & 0o111)) ? 'fm-perm-exec' : 'text-dim']">{{ e.permissions }}</td>
                <td class="text-dim">{{ fmtDate(e.modified) }}</td>
                <td>
                  <div class="fm-acts">
                    <button v-if="!e.is_dir" class="fm-act" :title="t('fm.download')" @click="downloadFile(e.path)"><i class="fa-solid fa-download"></i></button>
                    <button class="fm-act" :title="t('fm.rename')" @click="openRename(e.path)"><i class="fa-solid fa-pen"></i></button>
                    <button class="fm-act" :title="t('fm.properties')" @click="onStat(e.path)"><i class="fa-solid fa-circle-info"></i></button>
                    <button class="fm-act fm-act-danger" :title="t('common.delete')" @click="onDelete(e.path, e.is_dir)"><i class="fa-solid fa-trash"></i></button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <!-- Grid view -->
        <div v-else class="fm-listing">
          <div v-if="!entries.length" class="empty">{{ t('fm.emptyDir') }}</div>
          <div v-else class="fm-grid">
            <div v-for="e in entries" :key="e.path" :class="['fm-grid-item', { 'fm-openable': e.is_dir }]" @click="e.is_dir && openDir(e.path)">
              <div class="fm-grid-ico"><i :class="fileIcon(e)"></i></div>
              <div class="fm-grid-name" :title="e.name">{{ e.name }}</div>
              <div class="fm-grid-meta mono">{{ e.is_dir ? t('fm.folder') : fmtBytes(e.size) }}</div>
              <div class="fm-grid-acts" @click.stop>
                <button v-if="!e.is_dir" class="fm-act" :title="t('fm.download')" @click="downloadFile(e.path)"><i class="fa-solid fa-download"></i></button>
                <button class="fm-act" :title="t('fm.rename')" @click="openRename(e.path)"><i class="fa-solid fa-pen"></i></button>
                <button class="fm-act" :title="t('fm.properties')" @click="onStat(e.path)"><i class="fa-solid fa-circle-info"></i></button>
                <button class="fm-act fm-act-danger" :title="t('common.delete')" @click="onDelete(e.path, e.is_dir)"><i class="fa-solid fa-trash"></i></button>
              </div>
            </div>
          </div>
        </div>
      </template>
    </div>
  </div>

  <!-- Upload modal -->
  <div v-if="showUpload" class="modal-overlay" @click="(e) => { if (e.target === e.currentTarget) showUpload = false }">
    <div class="modal upload-modal">
      <h3>{{ t('fm.uploadTitle') }}</h3>
      <div class="upload-target">{{ t('fm.uploadTarget', { path: currentDir }) }}</div>

      <div
        :class="['upload-dropzone', { dragover: dropzoneDragover }]"
        @click="uploadInput?.click()"
        @drop="onDrop"
        @dragover="onDragover"
        @dragleave="onDragleave"
      >
        <i class="fa-solid fa-cloud-arrow-up"></i>
        <div>{{ t('fm.uploadDropzone') }}</div>
      </div>
      <input ref="uploadInput" type="file" multiple style="display:none" @change="onUploadPicked" />

      <div v-if="uploadQueue.length" class="upload-list">
        <div v-for="item in uploadQueue" :key="item.id" class="upload-item">
          <span class="fm-row-ico"><i class="fa-regular fa-file"></i></span>
          <span class="upload-item-name">{{ item.name }}</span>
          <span class="upload-item-size mono">{{ fmtBytes(item.size) }}</span>
          <div class="upload-progress">
            <div
              :class="['upload-progress-bar', { done: item.status === 'done', error: item.status === 'error' }]"
              :style="{ width: item.progress + '%' }"
            ></div>
          </div>
          <span :class="['upload-item-status', item.status]">
            {{ item.status === 'done' ? t('fm.uploadDone') : item.status === 'error' ? t('fm.uploadError') : item.status === 'uploading' ? item.progress + '%' : t('fm.uploadPending') }}
          </span>
          <button v-if="item.status === 'pending'" class="fm-act fm-act-danger" @click="removeUploadItem(item.id)"><i class="fa-solid fa-xmark"></i></button>
        </div>
      </div>

      <div class="upload-toolbar">
        <button class="btn-secondary btn-sm" @click="clearCompleted" v-if="uploadQueue.some((u) => u.status === 'done' || u.status === 'error')">{{ t('fm.uploadClear') }}</button>
        <span class="text-dim" style="font-size:12px;">{{ uploadQueue.length }} {{ t('fm.uploadPending') }}</span>
      </div>

      <div class="modal-actions">
        <button class="btn-secondary" @click="showUpload = false">{{ t('common.close') }}</button>
        <button @click="startUpload" :disabled="!uploadQueue.some((u) => u.status === 'pending')">{{ t('fm.uploadStart') }}</button>
      </div>
    </div>
  </div>

  <!-- Mkdir modal -->
  <div v-if="showMkdir" class="modal-overlay">
    <div class="modal">
      <h3>{{ t('fm.mkdir') }}</h3>
      <div class="field"><label>{{ t('fm.mkdirLabel') }}</label><input type="text" v-model="mkdirName" @keydown.enter="doMkdir" /></div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showMkdir = false">{{ t('common.cancel') }}</button>
        <button @click="doMkdir">{{ t('common.ok') }}</button>
      </div>
    </div>
  </div>

  <!-- Rename modal -->
  <div v-if="showRename" class="modal-overlay">
    <div class="modal">
      <h3>{{ t('fm.rename') }}</h3>
      <div class="field"><label>{{ t('fm.renameLabel') }}</label><input type="text" v-model="renameName" @keydown.enter="doRename" /></div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showRename = false">{{ t('common.cancel') }}</button>
        <button @click="doRename">{{ t('common.ok') }}</button>
      </div>
    </div>
  </div>

  <!-- Stat modal -->
  <div v-if="statInfo" class="modal-overlay">
    <div class="modal" style="max-width:560px;">
      <h3>{{ t('fm.statTitle', { name: statInfo.name }) }}</h3>
      <div class="fm-stat-grid">
        <div class="fm-stat-row"><div class="fm-stat-label">{{ t('fm.path') }}</div><div class="fm-stat-val mono">{{ statInfo.path }}</div></div>
        <div class="fm-stat-row"><div class="fm-stat-label">{{ t('common.type') }}</div><div class="fm-stat-val">{{ statInfo.is_dir ? t('fm.folder') : statInfo.is_symlink ? t('fm.symlink') : t('fm.file') }}</div></div>
        <div v-if="statInfo.symlink_target" class="fm-stat-row"><div class="fm-stat-label">{{ t('fm.target') }}</div><div class="fm-stat-val mono">{{ statInfo.symlink_target }}</div></div>
        <div class="fm-stat-row"><div class="fm-stat-label">{{ t('common.size') }}</div><div class="fm-stat-val mono">{{ statInfo.is_dir ? '—' : t('fm.sizeVal', { fmt: fmtBytes(statInfo.size), bytes: statInfo.size.toLocaleString() }) }}</div></div>
        <div class="fm-stat-row"><div class="fm-stat-label">{{ t('common.permissions') }}</div><div class="fm-stat-val"><span class="mono">{{ statInfo.permissions }}</span><button class="fm-act" style="margin-left:6px;" :title="t('fm.editPermissions')" @click="openChmod(statInfo)"><i class="fa-solid fa-pen"></i></button></div></div>
        <div class="fm-stat-row"><div class="fm-stat-label">{{ t('common.owner') }}</div><div class="fm-stat-val mono">{{ statInfo.user }} ({{ statInfo.uid }}) / {{ statInfo.group }} ({{ statInfo.gid }})<button class="fm-act" style="margin-left:6px;" :title="t('fm.editOwner')" @click="openChown(statInfo)"><i class="fa-solid fa-pen"></i></button></div></div>
        <div class="fm-stat-row"><div class="fm-stat-label">{{ t('fm.modified') }}</div><div class="fm-stat-val">{{ fmtDate(statInfo.modified) }}</div></div>
      </div>
      <div class="modal-actions"><button class="btn-secondary" @click="statInfo = null">{{ t('common.close') }}</button></div>
    </div>
  </div>

  <!-- Chmod modal -->
  <PermissionDialog
    v-if="showChmod"
    :title="`${t('fm.editPermissions')} — ${chmodInfo?.name || ''}`"
    :value="chmodMode"
    @confirm="doChmod"
    @close="showChmod = false"
  />

  <!-- Chown modal -->
  <div v-if="showChown" class="modal-overlay">
    <div class="modal">
      <h3>{{ t('fm.editOwner') }} — {{ chownInfo?.name }}</h3>
      <div class="field">
        <label>{{ t('common.user') }}</label>
        <select v-model="chownUid">
          <option value="">— {{ t('common.unknown') }} —</option>
          <option v-for="u in (chownAccounts?.users || [])" :key="u.id" :value="String(u.id)">{{ u.name }} ({{ u.id }})</option>
        </select>
      </div>
      <div class="field">
        <label>{{ t('common.group') }}</label>
        <select v-model="chownGid">
          <option value="">— {{ t('common.unknown') }} —</option>
          <option v-for="g in (chownAccounts?.groups || [])" :key="g.id" :value="String(g.id)">{{ g.name }} ({{ g.id }})</option>
        </select>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showChown = false">{{ t('common.cancel') }}</button>
        <button @click="doChown">{{ t('common.ok') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Layout */
.fm-wrap { display: flex; gap: 16px; height: calc(100vh - 170px); min-height: 420px; }
.fm-tree {
  width: 260px; flex-shrink: 0;
  background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--radius);
  display: flex; flex-direction: column; overflow: hidden;
}
.fm-tree-head { padding: 10px 14px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-dim); border-bottom: 1px solid var(--border); }
.fm-tree-body { overflow-y: auto; flex: 1; padding: 6px 0; }
.fm-tree-node { }
.fm-tree-row { display: flex; align-items: center; gap: 2px; cursor: pointer; }
.fm-tree-row:hover { background: var(--hover-bg); }
.fm-tree-arrow { width: 14px; text-align: center; color: var(--text-dim); font-size: 10px; user-select: none; flex-shrink: 0; }
.fm-tree-arrow:hover { color: var(--text); }
.fm-tree-name { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text-dim); padding: 4px 6px; min-width: 0; }
.fm-tree-name:hover { color: var(--text); }
.fm-tree-row.active .fm-tree-name { color: var(--accent); font-weight: 600; }
.fm-tree-ico { font-size: 13px; opacity: 0.8; }
.fm-tree-ico i { font-size: 13px; }
.fm-tree-arrow i { font-size: 11px; }
.fm-tree-children { }

.fm-main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.fm-toolbar { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 12px; flex-wrap: wrap; }
.fm-breadcrumb { display: flex; align-items: center; gap: 2px; font-size: 13px; flex-wrap: wrap; min-width: 0; }
.fm-crumb { color: var(--text-dim); cursor: pointer; padding: 2px 4px; border-radius: 4px; }
.fm-crumb:hover { color: var(--accent); background: var(--hover-bg); }
.fm-sep { color: var(--text-dim); opacity: 0.5; }
.fm-actions { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.fm-view-toggle { display: flex; gap: 4px; }

/* List view */
.fm-listing { flex: 1; overflow: auto; background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--radius); }
.fm-table { border-collapse: separate; border-spacing: 0; }
.fm-table td, .fm-table th { white-space: nowrap; }
.fm-table .fm-name-cell { white-space: normal; }
.fm-table th { position: sticky; top: 0; background: var(--bg-elev); z-index: 1; border-bottom: 1px solid var(--border); }
.fm-name-cell { }
.fm-cell-flex { display: flex; align-items: center; gap: 8px; min-width: 0; }
.fm-row-ico { font-size: 15px; opacity: 0.85; flex-shrink: 0; }
.fm-row-ico i { font-size: 15px; }
.fm-name-link { color: var(--text); cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.fm-name-link:hover { color: var(--accent); }
.fm-name { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.fm-acts { display: flex; gap: 2px; white-space: nowrap; justify-content: flex-end; }
.fm-act { display: inline-flex; align-items: center; justify-content: center; width: 26px; height: 26px; background: transparent; border: none; color: var(--text-dim); border-radius: 4px; cursor: pointer; font-size: 13px; padding: 0; }
.fm-act:hover { background: var(--hover-bg); color: var(--accent); }
.fm-act-danger:hover { color: var(--danger); }
.fm-perm-exec { color: var(--danger); }

/* Grid view */
.fm-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 12px; padding: 16px; }
.fm-grid-item { background: var(--bg-elev2); border: 1px solid var(--border); border-radius: var(--radius); padding: 14px 10px; text-align: center; transition: border-color 0.12s ease, background 0.12s ease; position: relative; }
.fm-grid-item:hover { border-color: var(--accent); }
.fm-grid-item.fm-openable { cursor: pointer; }
.fm-grid-ico { font-size: 34px; opacity: 0.8; margin-bottom: 8px; }
.fm-grid-ico i { font-size: 34px; }
.fm-grid-name { font-size: 13px; word-break: break-all; line-height: 1.3; margin-bottom: 4px; max-height: 34px; overflow: hidden; }
.fm-grid-meta { font-size: 11px; color: var(--text-dim); }
.fm-grid-acts { display: flex; justify-content: center; gap: 2px; margin-top: 8px; opacity: 0; transition: opacity 0.12s ease; }
.fm-grid-item:hover .fm-grid-acts { opacity: 1; }

/* Stat modal */
.fm-stat-grid { display: flex; flex-direction: column; gap: 2px; max-height: 60vh; overflow-y: auto; }
.fm-stat-row { display: grid; grid-template-columns: 90px 1fr; gap: 12px; padding: 6px 0; border-bottom: 1px solid var(--border); align-items: start; }
.fm-stat-label { color: var(--text-dim); font-size: 12px; }
.fm-stat-val { font-size: 13px; word-break: break-all; }


/* Upload manager */
.upload-modal { max-width: 560px; display: flex; flex-direction: column; max-height: 80vh; }
.upload-target { font-size: 12px; color: var(--text-dim); margin-bottom: 12px; word-break: break-all; }
.upload-dropzone {
  border: 2px dashed var(--border); border-radius: var(--radius); padding: 32px 20px;
  text-align: center; cursor: pointer; transition: border-color 0.15s, background 0.15s;
  color: var(--text-dim); font-size: 13px; margin-bottom: 12px;
}
.upload-dropzone:hover, .upload-dropzone.dragover { border-color: var(--accent); background: rgba(59,130,246,0.05); color: var(--text); }
.upload-dropzone i { font-size: 28px; margin-bottom: 8px; display: block; }
.upload-list { flex: 1; overflow-y: auto; max-height: 300px; margin-bottom: 12px; }
.upload-item { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-bottom: 1px solid var(--border); font-size: 13px; }
.upload-item-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.upload-item-size { font-size: 11px; color: var(--text-dim); flex-shrink: 0; }
.upload-item-status { font-size: 11px; flex-shrink: 0; width: 60px; text-align: right; }
.upload-item-status.done { color: var(--success); }
.upload-item-status.error { color: var(--danger); }
.upload-item-status.pending { color: var(--text-dim); }
.upload-item-status.uploading { color: var(--accent); }
.upload-progress { width: 80px; height: 4px; background: var(--bg-elev2); border-radius: 2px; overflow: hidden; flex-shrink: 0; }
.upload-progress-bar { height: 100%; background: var(--accent); transition: width 0.2s ease; }
.upload-progress-bar.done { background: var(--success); }
.upload-progress-bar.error { background: var(--danger); }
.upload-toolbar { display: flex; justify-content: space-between; align-items: center; gap: 8px; margin-bottom: 12px; }
.upload-badge {
  display: inline-flex; align-items: center; justify-content: center;
  min-width: 16px; height: 16px; padding: 0 4px; margin-left: 4px;
  font-size: 10px; font-weight: 700; color: #fff;
  background: var(--accent-emphasis); border-radius: 8px;
}
</style>
