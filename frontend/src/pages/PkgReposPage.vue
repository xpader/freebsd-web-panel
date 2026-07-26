<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const PRESETS = [
  {
    key: 'repoPresetOfficialLatest',
    filename: 'FreeBSD.conf',
    repos: [
      { name: 'FreeBSD-ports', url: 'pkg+https://pkg.freebsd.org/${ABI}/latest', mirror_type: 'srv', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg', enabled: true },
      { name: 'FreeBSD-ports-kmods', url: 'pkg+https://pkg.freebsd.org/${ABI}/kmods_latest_${VERSION_MINOR}', mirror_type: 'srv', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg', enabled: true },
    ],
    disables: [],
  },
  {
    key: 'repoPresetOfficialQuarterly',
    filename: 'FreeBSD.conf',
    repos: [
      { name: 'FreeBSD-ports', url: 'pkg+https://pkg.freebsd.org/${ABI}/quarterly', mirror_type: 'srv', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg', enabled: true },
      { name: 'FreeBSD-ports-kmods', url: 'pkg+https://pkg.freebsd.org/${ABI}/kmods_quarterly_${VERSION_MINOR}', mirror_type: 'srv', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg', enabled: true },
    ],
    disables: [],
  },
  {
    key: 'repoPresetUstc',
    filename: 'ustc.conf',
    repos: [
      { name: 'ustc-ports', url: 'https://mirrors.ustc.edu.cn/freebsd-pkg/${ABI}/quarterly', mirror_type: 'none', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg', enabled: true },
      { name: 'ustc-ports-kmods', url: 'https://mirrors.ustc.edu.cn/freebsd-pkg/${ABI}/kmods_quarterly_${VERSION_MINOR}', mirror_type: 'none', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg', enabled: true },
    ],
    disables: ['FreeBSD-ports', 'FreeBSD-ports-kmods'],
  },
];

const repoFiles = ref([]);
const loading = ref(true);
const error = ref('');

// Modal state
const showRepoModal = ref(false);
const editingRepo = ref(null); // { repo, file } when editing
const presetFilename = ref(null);
const form = ref({});
const userFiles = ref([]);

async function load() {
  if (!repoFiles.value.length) loading.value = true;
  error.value = '';
  try {
    repoFiles.value = await api.get('/api/pkg/repos');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

function totalRepos() {
  return repoFiles.value.reduce((s, f) => s + f.repos.length, 0);
}

function openAdd(filename) {
  editingRepo.value = null;
  presetFilename.value = filename || null;
  form.value = {
    name: '', url: '', enabled: true, mirror_type: 'NONE',
    signature_type: 'NONE', fingerprints: '', pubkey: '',
    priority: 0, ip_version: 0,
    fileSelect: '', filename: '',
  };
  userFiles.value = repoFiles.value.filter((f) => !f.is_system);
  showRepoModal.value = true;
}

function openEdit(file, repo) {
  editingRepo.value = { repo, file };
  form.value = {
    name: repo.name, url: repo.url, enabled: repo.enabled,
    mirror_type: (repo.mirror_type || 'NONE').toUpperCase(),
    signature_type: (repo.signature_type || 'NONE').toUpperCase(),
    fingerprints: repo.fingerprints || '', pubkey: repo.pubkey || '',
    priority: repo.priority || 0, ip_version: repo.ip_version || 0,
  };
  showRepoModal.value = true;
}

async function applyPreset(idx) {
  const p = PRESETS[idx];
  showRepoModal.value = false;

  const repoNames = p.repos.map(r => r.name).join(', ');
  let msg = t('pkg.repoPresetConfirmRepos', { repos: repoNames });
  if (p.disables.length) {
    msg += '\n' + t('pkg.repoPresetConfirmDisables', { repos: p.disables.join(', ') });
  }

  if (!await confirm(t('pkg.repoPresetConfirmTitle'), msg)) return;
  try {
    await api.post('/api/pkg/repos/apply_mirror', {
      filename: p.filename,
      repos: p.repos,
      disables: p.disables,
    });
    toast.toast(t('pkg.repoPresetApplied'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function saveRepo() {
  const repoBody = {
    name: form.value.name.trim(),
    url: form.value.url.trim(),
    enabled: form.value.enabled,
    mirror_type: form.value.mirror_type,
    signature_type: form.value.signature_type,
    fingerprints: form.value.fingerprints.trim() || null,
    pubkey: form.value.pubkey.trim() || null,
    priority: parseInt(form.value.priority) || 0,
    ip_version: parseInt(form.value.ip_version) || 0,
  };
  if (!repoBody.name || !repoBody.url) {
    await alert(t('common.fillRequired'), t('common.fillRequired'));
    return;
  }
  try {
    if (editingRepo.value) {
      await api.put(`/api/pkg/repos/${encodeURIComponent(editingRepo.value.repo.name)}`, {
        file: editingRepo.value.file.path, ...repoBody,
      });
      toast.toast(t('pkg.repoUpdateOk', { name: repoBody.name }));
    } else {
      let filename = form.value.fileSelect || form.value.filename.trim();
      if (!filename) {
        await alert(t('pkg.repoFileRequired'), t('pkg.repoFileRequired'));
        return;
      }
      await api.post('/api/pkg/repos', { filename, ...repoBody });
      toast.toast(t('pkg.repoCreateOk', { name: repoBody.name }));
    }
    showRepoModal.value = false;
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function toggleRepo(file, repo) {
  try {
    await api.put(`/api/pkg/repos/${encodeURIComponent(repo.name)}`, {
      file: file.path, url: repo.url, enabled: !repo.enabled,
      mirror_type: repo.mirror_type, signature_type: repo.signature_type,
      fingerprints: repo.fingerprints, pubkey: repo.pubkey,
      priority: repo.priority, ip_version: repo.ip_version,
    });
    toast.toast(t('pkg.repoUpdateOk', { name: repo.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function deleteRepo(file, repo) {
  if (repo.is_system_origin) {
    await alert(t('pkg.repoSystemNoDelete'), t('pkg.repoSystemNoDelete'));
    return;
  }
  if (!await confirm(t('pkg.repoDelete'), t('pkg.repoDeleteConfirm', { name: repo.name }))) return;
  try {
    await api.del(`/api/pkg/repos/${encodeURIComponent(repo.name)}?file=${encodeURIComponent(file.path)}`);
    toast.toast(t('pkg.repoDeleteOk', { name: repo.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// Refresh catalog task modal
const showRefresh = ref(false);
const refreshOutput = ref('');
const refreshDone = ref(false);

async function refreshCatalog() {
  let taskId;
  try {
    const res = await api.post('/api/pkg/repos/update', {});
    taskId = res.task_id;
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  refreshOutput.value = '';
  refreshDone.value = false;
  showRefresh.value = true;

  const token = sessionStorage.getItem('fwp_token');
  const url = `/api/tasks/${encodeURIComponent(taskId)}/stream?token=${encodeURIComponent(token)}`;
  const es = new EventSource(url);

  const finish = async (success) => {
    es.close();
    refreshDone.value = true;
    const label = success ? t('pkg.repoRefreshDone') : t('pkg.repoRefreshFailed');
    if (success) toast.toast(label);
    else await alert(t('pkg.repoRefreshFailed'), label);
  };

  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data.lines?.length) refreshOutput.value += data.lines.join('\n') + '\n';
      if (data.status && data.status !== 'running') finish(data.status === 'done');
    } catch {}
  };
  es.addEventListener('done', () => { es.close(); refreshDone.value = true; });
  es.onerror = () => {
    es.close();
    api.get(`/api/tasks/${encodeURIComponent(taskId)}`).then((task) => {
      if (task.status !== 'running') finish(task.status === 'done');
      else refreshDone.value = true;
    }).catch(() => { refreshDone.value = true; });
  };
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('nav.pkgRepos') }}</h1>
    <p>{{ t('pkg.repoSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <span class="text-dim">{{ t('pkg.repoFileCount', { n: repoFiles.length, m: totalRepos() }) }}</span>
    <div class="flex">
      <button @click="openAdd(null)"><i class="fa-solid fa-plus"></i> {{ t('pkg.repoAdd') }}</button>
      <button @click="refreshCatalog"><i class="fa-solid fa-rotate-right"></i> {{ t('pkg.repoRefresh') }}</button>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <div v-if="!repoFiles.length" class="empty">{{ t('pkg.repoNoRepos') }}</div>
    <div v-for="file in repoFiles" :key="file.filename" class="card" style="padding:0; margin-bottom:16px;">
      <div style="padding:12px 16px; display:flex; align-items:center; gap:8px; border-bottom:1px solid var(--border);">
        <i class="fa-solid fa-file-lines"></i>
        <strong>{{ file.filename }}</strong>
        <span v-if="file.is_system" class="badge badge-dim">{{ t('pkg.repoSystem') }}</span>
        <span v-else class="badge badge-success">{{ t('pkg.repoCustom') }}</span>
        <span class="text-dim" style="font-size:12px; flex:1;">{{ file.path }}</span>
        <button class="btn-secondary btn-sm" @click="openAdd(file.filename)"><i class="fa-solid fa-plus"></i> {{ t('pkg.repoAdd') }}</button>
      </div>
      <table v-if="file.repos.length">
        <thead><tr>
          <th>{{ t('pkg.repoName') }}</th><th>{{ t('pkg.repoUrl') }}</th><th>{{ t('pkg.repoEnabled') }}</th>
          <th>{{ t('pkg.repoPriority') }}</th><th>{{ t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="r in file.repos" :key="r.name">
            <td class="mono"><strong>{{ r.name }}</strong>
              <span v-if="r.is_system_origin" class="badge badge-dim">{{ t('pkg.repoSystem') }}</span>
            </td>
            <td class="mono text-dim" style="max-width:360px; overflow:hidden; text-overflow:ellipsis;">{{ r.url }}</td>
            <td>
              <span v-if="r.enabled" class="badge badge-success">{{ t('pkg.repoEnabledYes') }}</span>
              <span v-else class="badge badge-dim">{{ t('pkg.repoEnabledNo') }}</span>
            </td>
            <td class="mono">{{ r.priority || 0 }}</td>
            <td>
              <div class="btn-group">
                <button class="btn-secondary btn-sm" @click="toggleRepo(file, r)">{{ r.enabled ? t('common.disable') : t('common.enable') }}</button>
                <button class="btn-secondary btn-sm" @click="openEdit(file, r)">{{ t('common.edit') }}</button>
                <button v-if="!r.is_system_origin" class="btn-danger btn-sm" @click="deleteRepo(file, r)">{{ t('common.delete') }}</button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="empty">{{ t('pkg.repoNoRepos') }}</div>
    </div>
  </template>

  <!-- Add/Edit modal -->
  <div v-if="showRepoModal" class="modal-overlay">
    <div class="modal" style="max-width:600px;">
      <h3>{{ editingRepo ? t('pkg.repoEdit') : t('pkg.repoAdd') }}</h3>

      <div v-if="!editingRepo" style="margin-bottom:16px;">
        <div class="card-title" style="font-size:13px;">{{ t('pkg.repoPresetTitle') }}</div>
        <div class="text-dim" style="font-size:12px; margin-top:2px;">{{ t('pkg.repoPresetHint') }}</div>
        <div class="flex" style="flex-wrap:wrap; gap:6px; margin-top:6px;">
          <button v-for="(p, i) in PRESETS" :key="i" class="btn-secondary btn-sm" @click="applyPreset(i)">{{ t('pkg.' + p.key) }}</button>
        </div>
      </div>

      <div class="repo-form">
        <div v-if="!editingRepo" class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoTargetFile') }}</label>
          <div class="repo-form-field">
            <select v-if="userFiles.length" v-model="form.fileSelect" class="filter-input" style="width:auto;">
              <option value="">— {{ t('pkg.repoNewFile') }} —</option>
              <option v-for="f in userFiles" :key="f.filename" :value="f.filename">{{ f.filename }}</option>
            </select>
            <input type="text" v-show="!form.fileSelect" v-model="form.filename" class="filter-input" placeholder="e.g. FreeBSD.conf" style="width:180px;" />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoName') }}</label>
          <div class="repo-form-field">
            <input type="text" v-model="form.name" class="filter-input" :placeholder="t('pkg.repoNamePh')" style="width:100%;" :readonly="editingRepo && editingRepo.repo.is_system_origin" />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoUrl') }}</label>
          <div class="repo-form-field">
            <input type="text" v-model="form.url" class="filter-input" :placeholder="t('pkg.repoUrlPh')" style="width:100%;" />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoEnabled') }}</label>
          <div class="repo-form-field">
            <label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
              <input type="checkbox" v-model="form.enabled" /> {{ t('pkg.repoEnabledYes') }}
            </label>
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoMirrorType') }}</label>
          <div class="repo-form-field">
            <select v-model="form.mirror_type" class="filter-input" style="width:auto;">
              <option value="NONE">NONE</option><option value="SRV">SRV</option><option value="HTTP">HTTP</option>
            </select>
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoSignatureType') }}</label>
          <div class="repo-form-field">
            <select v-model="form.signature_type" class="filter-input" style="width:auto;">
              <option value="NONE">NONE</option><option value="FINGERPRINTS">FINGERPRINTS</option><option value="PUBKEY">PUBKEY</option>
            </select>
          </div>
        </div>
        <div v-if="form.signature_type === 'FINGERPRINTS'" class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoFingerprints') }}</label>
          <div class="repo-form-field">
            <input type="text" v-model="form.fingerprints" class="filter-input" placeholder="/usr/share/keys/pkg" style="width:100%;" />
          </div>
        </div>
        <div v-if="form.signature_type === 'PUBKEY'" class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoPubkey') }}</label>
          <div class="repo-form-field">
            <input type="text" v-model="form.pubkey" class="filter-input" placeholder="/usr/share/keys/pubkey.pem" style="width:100%;" />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoPriority') }}</label>
          <div class="repo-form-field">
            <input type="number" v-model="form.priority" class="filter-input" style="width:80px;" />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">{{ t('pkg.repoIpVersion') }}</label>
          <div class="repo-form-field">
            <select v-model="form.ip_version" class="filter-input" style="width:auto;">
              <option value="0">{{ t('common.default') }}</option><option value="4">IPv4</option><option value="6">IPv6</option>
            </select>
          </div>
        </div>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showRepoModal = false">{{ t('common.cancel') }}</button>
        <button @click="saveRepo">{{ t('common.save') }}</button>
      </div>
    </div>
  </div>

  <!-- Refresh catalog modal -->
  <div v-if="showRefresh" class="modal-overlay">
    <div class="modal" style="max-width:680px;">
      <h3><span v-if="!refreshDone" class="spinner"></span> {{ t('pkg.repoRefreshing') }}</h3>
      <div style="max-height:400px; overflow-y:auto; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius); padding:12px; margin-bottom:12px; font-family:monospace; font-size:12px; white-space:pre-wrap; word-break:break-all;">{{ refreshOutput }}</div>
      <div class="modal-actions">
        <button class="btn-secondary" :disabled="!refreshDone" @click="showRefresh = false">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>
