<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const pkgFilter = ref('all');
const allPackages = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const search = ref('');

const filtered = computed(() => {
  const q = search.value.toLowerCase();
  if (!q) return allPackages.value;
  return allPackages.value.filter((p) =>
    p.name.toLowerCase().includes(q) || (p.comment || '').toLowerCase().includes(q) || (p.origin || '').toLowerCase().includes(q)
  );
});

async function load() {
  if (!allPackages.value.length) loading.value = true;
  refreshing.value = true;
  search.value = '';
  try {
    allPackages.value = await api.get(`/api/pkg/packages?filter=${pkgFilter.value}`);
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

function setFilter(v) {
  pkgFilter.value = v;
  load();
}

// Install modal
const showInstall = ref(false);
const installSearch = ref('');
const installResults = ref(null);
const installLoading = ref(false);
const installedNames = ref(new Set());

async function doRemoteSearch() {
  const q = installSearch.value.trim();
  if (!q) return;
  installLoading.value = true;
  installResults.value = null;
  try {
    const results = await api.get(`/api/pkg/search?q=${encodeURIComponent(q)}`);
    const installed = await api.get('/api/pkg/packages');
    installedNames.value = new Set(installed.map((p) => p.name));
    installResults.value = results;
  } catch (e) {
    installResults.value = [];
  } finally {
    installLoading.value = false;
  }
}

async function doInstall(name) {
  showInstall.value = false;
  // Preview + confirm
  let preview;
  try {
    preview = await api.post('/api/pkg/preview', { action: 'install', packages: [name] });
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  const deps = preview.install.filter((n) => n !== name);
  let msg = '';
  if (deps.length) {
    msg = t('pkg.willInstallDeps', { n: deps.length }) + '\n' + deps.join('\n');
  } else if (preview.install.length === 0) {
    msg = t('pkg.alreadyInstalled');
  } else {
    msg = t('pkg.noDepsToInstall');
  }
  if (!await confirmDialogCustom(t('pkg.installConfirm', { name }), msg)) return;

  // Start task
  let taskId;
  try {
    const res = await api.post('/api/pkg/install', { packages: [name] });
    taskId = res.task_id;
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  showTaskModal('install', [name], taskId);
}

async function doDelete(name) {
  let preview;
  try {
    preview = await api.post('/api/pkg/preview', { action: 'delete', packages: [name] });
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  const deps = preview.delete.filter((n) => n !== name);
  let msg = '';
  if (deps.length) {
    msg = t('pkg.willDeleteDeps', { n: deps.length }) + '\n' + deps.join('\n');
  } else {
    msg = t('pkg.noDepsToDelete');
  }
  if (!await confirmDialogCustom(t('pkg.deleteConfirm', { name }), msg)) return;

  let taskId;
  try {
    const res = await api.post('/api/pkg/delete', { packages: [name] });
    taskId = res.task_id;
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  showTaskModal('delete', [name], taskId);
}

// Simple confirm using ui store
import { ui } from '../stores/ui.js';
function confirmDialogCustom(title, message) {
  return ui.showDialog({ type: 'confirm', title, message });
}

// Task output modal
const showTask = ref(false);
const taskAction = ref('');
const taskPackages = ref('');
const taskOutput = ref('');
const taskDone = ref(false);
const taskSuccess = ref(false);
let taskEs = null;

function showTaskModal(action, packages, taskId) {
  taskAction.value = action;
  taskPackages.value = packages.join(', ');
  taskOutput.value = '';
  taskDone.value = false;
  taskSuccess.value = false;
  showTask.value = true;

  const token = sessionStorage.getItem('fwp_token');
  const url = `/api/pkg/tasks/${encodeURIComponent(taskId)}/stream?token=${encodeURIComponent(token)}`;
  const es = new EventSource(url);
  taskEs = es;

  const finish = async (success, pkgNames) => {
    es.close();
    taskDone.value = true;
    taskSuccess.value = success;
    const nameStr = pkgNames || packages.join(', ');
    const doneLabel = success
      ? t(action === 'install' ? 'pkg.installDone' : 'pkg.deleteDone', { name: nameStr })
      : t(action === 'install' ? 'pkg.installFailed' : 'pkg.deleteFailed', { name: nameStr });
    if (success) {
      taskOutput.value += `\n[${t('common.done')}]\n`;
      toast.toast(doneLabel);
    } else {
      await alert(doneLabel, doneLabel);
    }
    load();
  };

  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data.lines && data.lines.length) {
        taskOutput.value += data.lines.join('\n') + '\n';
      }
      if (data.status && data.status !== 'running') {
        const pkgNames = Array.isArray(data.packages) ? data.packages.join(', ') : (data.packages || '');
        finish(data.status === 'done', pkgNames);
      }
    } catch {}
  };
  es.addEventListener('done', () => { es.close(); taskDone.value = true; });
  es.onerror = () => {
    es.close();
    api.get(`/api/pkg/tasks/${encodeURIComponent(taskId)}`).then((task) => {
      if (task.status !== 'running') {
        finish(task.status === 'done', task.packages.join(', '));
      } else {
        taskDone.value = true;
      }
    }).catch(() => { taskDone.value = true; });
  };
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('nav.packages') }}</h1>
    <p>{{ t('pkg.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <div class="filter-group">
      <button :class="['filter-btn', { active: pkgFilter === 'all' }]" @click="setFilter('all')">{{ t('common.all') }}</button>
      <button :class="['filter-btn', { active: pkgFilter === 'manual' }]" @click="setFilter('manual')">{{ t('pkg.manual') }}</button>
    </div>
    <input type="text" v-model="search" class="filter-input" :placeholder="t('pkg.filterPh')" />
    <span class="text-dim">{{ t('pkg.count', { n: filtered.length }) }}</span>
    <div class="flex">
      <button @click="showInstall = true"><i class="fa-solid fa-download"></i> {{ t('pkg.installBtn') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th><th>{{ t('pkg.version') }}</th><th>{{ t('common.description') }}</th>
        <th>{{ t('common.size') }}</th><th>{{ t('common.status') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="6" class="empty">{{ t('pkg.noPackages') }}</td></tr>
        <tr v-for="p in filtered" :key="p.name">
          <td class="mono"><strong><a :href="'#/pkg/' + p.name">{{ p.name }}</a></strong></td>
          <td class="mono text-dim">{{ p.version }}</td>
          <td><div class="cell-wrap">{{ p.comment || '—' }}</div></td>
          <td class="mono">{{ p.size }}</td>
          <td>
            <span v-if="p.automatic" class="badge badge-dim">{{ t('pkg.automatic') }}</span>
            <span v-else class="badge badge-success">{{ t('pkg.manual') }}</span>
          </td>
          <td><button class="btn-secondary btn-sm" @click="doDelete(p.name)">{{ t('common.delete') }}</button></td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Install search modal -->
  <div v-if="showInstall" class="modal-overlay">
    <div class="modal" style="max-width:680px;">
      <h3><i class="fa-solid fa-download"></i> {{ t('pkg.installTitle') }}</h3>
      <div class="field" style="margin-bottom:12px;">
        <div class="flex">
          <input type="text" v-model="installSearch" class="filter-input" style="flex:1;" :placeholder="t('pkg.searchRemotePh')" @keydown.enter.prevent="doRemoteSearch" />
          <button @click="doRemoteSearch"><i class="fa-solid fa-magnifying-glass"></i> {{ t('pkg.searchBtn') }}</button>
        </div>
      </div>
      <div style="max-height:360px; overflow-y:auto;">
        <div v-if="installLoading" class="empty" style="padding:20px;"><span class="spinner"></span> {{ t('common.loading') }}</div>
        <div v-else-if="!installResults" class="empty" style="padding:20px;">{{ t('pkg.searchHint') }}</div>
        <div v-else-if="!installResults.length" class="empty" style="padding:20px;">{{ t('pkg.noSearchResults') }}</div>
        <table v-else style="width:100%;">
          <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('common.description') }}</th><th>{{ t('common.size') }}</th><th></th></tr></thead>
          <tbody>
            <tr v-for="r in installResults" :key="r.name">
              <td class="mono"><strong>{{ r.name }}</strong><br><span class="text-dim" style="font-size:11px;">{{ r.version }}</span></td>
              <td><div class="cell-wrap">{{ r.comment }}</div></td>
              <td class="mono text-dim">{{ r.size }}</td>
              <td style="white-space:nowrap;">
                <span v-if="installedNames.has(r.name)" class="badge badge-dim">{{ t('pkg.installedBadge') }}</span>
                <button v-else class="btn-secondary btn-sm" @click="doInstall(r.name)">{{ t('pkg.installBtn') }}</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showInstall = false">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>

  <!-- Task output modal -->
  <div v-if="showTask" class="modal-overlay">
    <div class="modal" style="max-width:680px;">
      <h3>
        <span v-if="!taskDone" class="spinner"></span>
        {{ taskAction === 'install' ? t('pkg.installing') : t('pkg.deleting') }} {{ taskPackages }}
      </h3>
      <div style="max-height:400px; overflow-y:auto; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius); padding:12px; margin-bottom:12px; font-family:monospace; font-size:12px; white-space:pre-wrap; word-break:break-all;">{{ taskOutput }}</div>
      <div class="modal-actions">
        <button class="btn-secondary" :disabled="!taskDone" @click="showTask = false">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>
