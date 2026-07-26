<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import SearchInput from '../components/ui/SearchInput.vue';
import TaskConsole from '../components/ui/TaskConsole.vue';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const previewing = ref(null);

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

async function doUpgrade(name) {
  previewing.value = { action: 'upgrade', name };
  let preview;
  try {
    preview = await api.post('/api/pkg/preview', { action: 'upgrade', packages: [name] });
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    previewing.value = null;
    return;
  }
  previewing.value = null;
  if (!preview.upgrade.length && !preview.install.length && !preview.delete.length) {
    toast.toast(t('pkg.noUpgrades'));
    return;
  }
  let msg = '';
  if (preview.upgrade.length) {
    msg = t('pkg.willUpgrade', { n: preview.upgrade.length }) + '\n' + preview.upgrade.join('\n');
  }
  if (preview.install.length) {
    msg += '\n' + t('pkg.willInstallDeps', { n: preview.install.length }) + '\n' + preview.install.join('\n');
  }
  if (preview.delete.length) {
    msg += '\n' + t('pkg.willDeleteDeps', { n: preview.delete.length }) + '\n' + preview.delete.join('\n');
  }
  if (!await confirmDialogCustom(t('pkg.upgradeConfirmTitle'), msg)) return;

  let taskId;
  try {
    const res = await api.post('/api/pkg/upgrade', { packages: [name] });
    taskId = res.task_id;
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  showTaskModal('upgrade', [name], taskId);
}

async function doAutoremove() {
  previewing.value = { action: 'autoremove' };
  let preview;
  try {
    preview = await api.post('/api/pkg/preview', { action: 'autoremove' });
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    previewing.value = null;
    return;
  }
  previewing.value = null;
  if (!preview.delete.length) {
    toast.toast(t('pkg.noAutoremove'));
    return;
  }
  let msg = t('pkg.willAutoremove', { n: preview.delete.length }) + '\n' + preview.delete.join('\n');
  if (!await confirmDialogCustom(t('pkg.autoremoveConfirmTitle'), msg)) return;

  let taskId;
  try {
    const res = await api.post('/api/pkg/autoremove');
    taskId = res.task_id;
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  showTaskModal('autoremove', [], taskId);
}

// Task output modal
const showTask = ref(false);
const taskAction = ref('');
const taskPackages = ref('');
const taskDone = ref(false);
const activeTaskId = ref('');

const taskTitle = computed(() => {
  const titles = {
    install: t('pkg.installing'),
    delete: t('pkg.deleting'),
    upgrade: t('pkg.upgrading'),
    autoremove: t('pkg.autoremoving'),
  };
  return titles[taskAction.value] || taskAction.value;
});

function showTaskModal(action, packages, taskId) {
  taskAction.value = action;
  taskPackages.value = packages.join(', ');
  taskDone.value = false;
  activeTaskId.value = taskId;
  showTask.value = true;
}

async function onTaskDone({ success }) {
  taskDone.value = true;
  const labels = {
    install: { ok: 'pkg.installDone', fail: 'pkg.installFailed' },
    delete: { ok: 'pkg.deleteDone', fail: 'pkg.deleteFailed' },
    upgrade: { ok: 'pkg.upgradeDone', fail: 'pkg.upgradeFailed' },
    autoremove: { ok: 'pkg.autoremoveDone', fail: 'pkg.autoremoveFailed' },
  };
  const l = labels[taskAction.value] || labels.install;
  const doneLabel = t(success ? l.ok : l.fail, { name: taskPackages.value });
  if (success) {
    toast.toast(doneLabel);
  } else {
    await alert(doneLabel, doneLabel);
  }
  load();
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
    <SearchInput v-model="search" :placeholder="t('pkg.filterPh')" />
    <span class="text-dim">{{ t('pkg.count', { n: filtered.length }) }}</span>
    <div class="flex">
      <button @click="doAutoremove" :disabled="!!previewing"><span v-if="previewing?.action === 'autoremove'" class="spinner"></span><i v-else class="fa-solid fa-broom"></i> {{ previewing?.action === 'autoremove' ? t('pkg.checking') : t('pkg.autoremoveBtn') }}</button>
      <button @click="showInstall = true"><i class="fa-solid fa-download"></i> {{ t('pkg.installBtn') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <div class="table-wrap">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th><th>{{ t('pkg.version') }}</th><th>{{ t('common.description') }}</th>
        <th>{{ t('common.size') }}</th><th>{{ t('common.status') }}</th><th style="width:1%; white-space:nowrap;">{{ t('common.actions') }}</th>
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
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" :disabled="!!previewing" @click="doUpgrade(p.name)"><span v-if="previewing?.action === 'upgrade' && previewing?.name === p.name" class="spinner"></span>{{ previewing?.action === 'upgrade' && previewing?.name === p.name ? t('pkg.checking') : t('pkg.upgradeBtn') }}</button>
              <button class="btn-secondary btn-sm" :disabled="!!previewing" @click="doDelete(p.name)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
    </div>
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
        {{ taskTitle }} {{ taskPackages }}
      </h3>
      <TaskConsole :task-id="activeTaskId" style="margin-bottom:12px;" @done="onTaskDone" />
      <div class="modal-actions">
        <button class="btn-secondary" :disabled="!taskDone" @click="showTask = false">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>
