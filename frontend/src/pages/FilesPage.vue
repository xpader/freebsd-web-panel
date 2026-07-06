<script setup>
import { ref, reactive, onMounted, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { api, authFetch } from '../lib/api.js';
import { fmtBytes, fmtDate, permStringFull, octStr } from '../lib/format.js';
import { useToast, useConfirm, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const confirm = useConfirm();
const alert = useAlert();

const ROOT = '/';
const currentDir = ref('/root');
const viewMode = ref(localStorage.getItem('fwp_fm_view') || 'list');
const expanded = reactive(new Set([ROOT]));
const treeChildren = reactive(new Map());
const entries = ref([]);
const loading = ref(true);

// Modals
const showMkdir = ref(false);
const mkdirName = ref('');
const showRename = ref(false);
const renamePath = ref('');
const renameName = ref('');
const statInfo = ref(null);
const showChmod = ref(false);
const chmodInfo = ref(null);
const chmodMode = ref(0);
const showChown = ref(false);
const chownInfo = ref(null);
const chownAccounts = ref(null);
const chownUid = ref('');
const chownGid = ref('');

function fileIcon(e) {
  if (e.is_dir) return 'fa-solid fa-folder';
  if (e.is_symlink) return 'fa-solid fa-link';
  const ext = (e.name.split('.').pop() || '').toLowerCase();
  const map = {
    txt: 'fa-regular fa-file-lines', log: 'fa-regular fa-file-lines', md: 'fa-regular fa-file-lines',
    png: 'fa-regular fa-file-image', jpg: 'fa-regular fa-file-image', jpeg: 'fa-regular fa-file-image',
    gif: 'fa-regular fa-file-image', webp: 'fa-regular fa-file-image', svg: 'fa-regular fa-file-image',
    mp4: 'fa-regular fa-file-video', mkv: 'fa-regular fa-file-video', avi: 'fa-regular fa-file-video',
    mp3: 'fa-regular fa-file-audio', wav: 'fa-regular fa-file-audio',
    zip: 'fa-regular fa-file-zipper', gz: 'fa-regular fa-file-zipper', tar: 'fa-regular fa-file-zipper', xz: 'fa-regular fa-file-zipper', '7z': 'fa-regular fa-file-zipper',
    pdf: 'fa-regular fa-file-pdf',
    sh: 'fa-regular fa-file-code', py: 'fa-regular fa-file-code', js: 'fa-regular fa-file-code', rs: 'fa-regular fa-file-code', c: 'fa-regular fa-file-code', json: 'fa-regular fa-file-code',
  };
  return map[ext] || 'fa-regular fa-file';
}

function pathDepth(path) {
  if (path === ROOT) return 0;
  return path.split('/').filter(Boolean).length;
}

function joinPath(dir, name) {
  if (dir === ROOT) return ROOT + name;
  return dir + '/' + name;
}

async function fetchDirs(path) {
  const list = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
  return list.filter((e) => e.is_dir);
}

async function ensureAncestors(path) {
  const parts = path.split('/').filter(Boolean);
  let cur = '';
  for (const part of parts) {
    cur = cur + '/' + part;
    if (!treeChildren.has(cur)) {
      try { treeChildren.set(cur, await fetchDirs(cur)); } catch { treeChildren.set(cur, []); }
    }
    expanded.add(cur);
  }
  expanded.add(ROOT);
}

function getTreeChildren(path) {
  return treeChildren.get(path);
}

async function toggleExpand(path) {
  if (!treeChildren.has(path)) {
    try { treeChildren.set(path, await fetchDirs(path)); } catch { treeChildren.set(path, []); }
  }
  if (expanded.has(path)) expanded.delete(path);
  else expanded.add(path);
}

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

function breadcrumbParts() {
  return currentDir.value.split('/').filter(Boolean);
}

function breadcrumbPath(idx) {
  return '/' + breadcrumbParts().slice(0, idx + 1).join('/');
}

function invalidateTree(path) {
  for (const key of [...treeChildren.keys()]) {
    if (key === path || key.startsWith(path + '/')) treeChildren.delete(key);
  }
}

async function refreshTree() {
  const parts = currentDir.value.split('/').filter(Boolean);
  const paths = [ROOT];
  let cur = '';
  for (const p of parts) { cur = cur + '/' + p; paths.push(cur); }
  for (const p of paths) {
    try { treeChildren.set(p, await fetchDirs(p)); } catch {}
  }
}

// Upload
const uploadInput = ref(null);

async function onUploadPicked(ev) {
  const files = [...ev.target.files];
  ev.target.value = '';
  if (!files.length) return;
  for (const file of files) {
    try {
      const url = `/api/files/upload?path=${encodeURIComponent(currentDir.value)}&filename=${encodeURIComponent(file.name)}`;
      const res = await authFetch(url, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: file });
      if (!res.ok) throw { message: `Upload failed (${res.status})` };
      toast.toast(t('fm.uploaded', { name: file.name }));
    } catch (err) {
      await alert(t('fm.uploadFailed', { name: file.name, msg: '' }), t('fm.uploadFailed', { name: file.name, msg: err.message || '' }));
    }
  }
  await loadListing(currentDir.value);
}

async function downloadFile(path) {
  try {
    const res = await authFetch(`/api/files/download?path=${encodeURIComponent(path)}`);
    if (res.status === 401) { sessionStorage.removeItem('fwp_token'); return; }
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
    invalidateTree(currentDir.value);
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
    invalidateTree(parent);
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
    invalidateTree(path);
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
  chmodMode.value = info.mode & 0o7777;
  showChmod.value = true;
}

async function doChmod() {
  try {
    await api.put(`/api/files/chmod?path=${encodeURIComponent(chmodInfo.value.path)}`, { mode: chmodMode.value });
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

function setBit(bit, checked) {
  if (checked) chmodMode.value |= bit;
  else chmodMode.value &= ~bit;
}

onMounted(async () => {
  if (!treeChildren.has(ROOT)) treeChildren.set(ROOT, await fetchDirs(ROOT));
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
            <template v-for="d in (getTreeChildren(ROOT) || [])" :key="d.path">
              <div class="fm-tree-node">
                <div :class="['fm-tree-row', { active: currentDir === d.path }]" :style="{ paddingLeft: pathDepth(d.path) * 14 + 6 + 'px' }" @click="openDir(d.path)">
                  <span class="fm-tree-arrow" @click.stop="toggleExpand(d.path)">
                    <i v-if="expanded.has(d.path)" class="fa-solid fa-caret-down"></i>
                    <i v-else class="fa-solid fa-caret-right"></i>
                  </span>
                  <span class="fm-tree-name"><span class="fm-tree-ico"><i class="fa-solid fa-folder"></i></span>{{ d.path.split('/').filter(Boolean).pop() }}</span>
                </div>
              </div>
            </template>
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
            <button class="btn-secondary btn-sm" @click="uploadInput?.click()"><i class="fa-solid fa-upload"></i> {{ t('fm.upload') }}</button>
            <button class="btn-secondary btn-sm" @click="openMkdir"><i class="fa-solid fa-folder-plus"></i> {{ t('fm.mkdir') }}</button>
            <div class="fm-view-toggle">
              <button :class="['btn-secondary', 'btn-sm', { 'active-range': viewMode === 'list' }]" @click="setView('list')"><i class="fa-solid fa-list"></i> {{ t('fm.listView') }}</button>
              <button :class="['btn-secondary', 'btn-sm', { 'active-range': viewMode === 'grid' }]" @click="setView('grid')"><i class="fa-solid fa-table-cells"></i> {{ t('fm.gridView') }}</button>
            </div>
          </div>
        </div>

        <input ref="uploadInput" type="file" multiple style="display:none" @change="onUploadPicked" />

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
                <td :class="['mono', (e.mode & 0o111) ? 'fm-perm-exec' : 'text-dim']">{{ e.permissions }}</td>
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
  <div v-if="showChmod" class="modal-overlay">
    <div class="modal">
      <h3>{{ t('fm.editPermissions') }} — {{ chmodInfo?.name }}</h3>
      <div class="fm-perm-grid">
        <div v-for="row in [
          { label: t('common.owner'), bits: { r: 0o400, w: 0o200, x: 0o100 }, special: { s: 0o4000, label: 'setuid' } },
          { label: t('common.group'), bits: { r: 0o040, w: 0o020, x: 0o010 }, special: { s: 0o2000, label: 'setgid' } },
          { label: t('fm.other'), bits: { r: 0o004, w: 0o002, x: 0o001 }, special: { s: 0o1000, label: 'sticky' } },
        ]" :key="row.label" class="fm-perm-row">
          <span class="fm-perm-who">{{ row.label }}</span>
          <label class="fm-perm-check"><input type="checkbox" :checked="chmodMode & row.bits.r" @change="setBit(row.bits.r, $event.target.checked)" />{{ t('fm.read') }}</label>
          <label class="fm-perm-check"><input type="checkbox" :checked="chmodMode & row.bits.w" @change="setBit(row.bits.w, $event.target.checked)" />{{ t('fm.write') }}</label>
          <label class="fm-perm-check"><input type="checkbox" :checked="chmodMode & row.bits.x" @change="setBit(row.bits.x, $event.target.checked)" />{{ t('fm.execute') }}</label>
          <label class="fm-perm-check"><input type="checkbox" :checked="chmodMode & row.special.s" @change="setBit(row.special.s, $event.target.checked)" />{{ row.special.label }}</label>
        </div>
      </div>
      <div class="fm-perm-preview">
        <span class="text-dim">{{ t('common.permissions') }}:</span>
        <span class="mono">{{ permStringFull(chmodMode) }}</span>
        <span class="text-dim">{{ t('fm.octalMode') }}:</span>
        <span class="mono">{{ octStr(chmodMode) }}</span>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showChmod = false">{{ t('common.cancel') }}</button>
        <button @click="doChmod">{{ t('common.ok') }}</button>
      </div>
    </div>
  </div>

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
