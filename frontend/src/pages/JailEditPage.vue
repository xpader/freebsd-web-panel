<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter, useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import FilePicker from '../components/ui/FilePicker.vue';
import SectionCard from '../components/ui/SectionCard.vue';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const router = useRouter();
const route = useRoute();
const toast = useToast();
const alert = useAlert();

const name = route.params.name;
const jail = ref(null);
const error = ref('');
const submitting = ref(false);
const form = ref({});
const activeTab = ref('basic');
const autoStart = ref(false);

const pickerTarget = ref(null);
function openPicker(target) {
  pickerTarget.value = target;
}
function onPickerSelect(path) {
  if (pickerTarget.value === '__fstab') {
    fstabEditing.value.fs_spec = path;
  } else {
    form.value[pickerTarget.value] = path;
  }
  pickerTarget.value = null;
}

// ── Fstab management ──
const fstabEntries = ref([]);
const fstabEditing = ref(null);
const fstabEditingIdx = ref(null);
const showFstabModal = ref(false);
const FSTAB_TYPES = ['nullfs', 'tmpfs', 'unionfs'];

async function loadFstab() {
  try {
    fstabEntries.value = await api.get(`/api/jails/${encodeURIComponent(name)}/fstab`);
  } catch {
    fstabEntries.value = [];
  }
}

function fstabAdd() {
  fstabEditing.value = { fs_spec: '', fs_file: '', fs_vfstype: 'nullfs', fs_mntops: 'rw', fs_freq: '0', fs_passno: '0' };
  fstabEditingIdx.value = null;
}
function fstabEdit(idx) {
  fstabEditing.value = { ...fstabEntries.value[idx] };
  fstabEditingIdx.value = idx;
}
function fstabDelete(idx) {
  fstabEntries.value.splice(idx, 1);
  saveFstab();
}
function fstabSaveEntry() {
  const e = fstabEditing.value;
  if (!e || !e.fs_spec || !e.fs_file) return;
  if (fstabEditingIdx.value !== null) {
    fstabEntries.value[fstabEditingIdx.value] = { ...e };
  } else {
    fstabEntries.value.push({ ...e });
  }
  fstabEditing.value = null;
  fstabEditingIdx.value = null;
  saveFstab();
}
async function saveFstab() {
  try {
    await api.put(`/api/jails/${encodeURIComponent(name)}/fstab`, { entries: fstabEntries.value });
  } catch (e) {
    toast.toast(e.message || t('common.operationFailed'), 'error');
  }
}

const JAIL_READONLY = new Set([
  'jid', 'dying', 'lastjid', 'children.cur',
  'osrelease', 'osreldate', 'cpuset.id',
  'ip4.saddrsel', 'ip6.saddrsel',
]);

const TABS = [
  { key: 'basic', titleKey: 'jails.basicInfo' },
  { key: 'network', titleKey: 'common.network' },
  { key: 'exec', titleKey: 'jails.editExec' },
  { key: 'mount', titleKey: 'jails.editMount' },
  { key: 'security', titleKey: 'jails.security' },
  { key: 'misc', titleKey: 'jails.editMisc' },
];
const tabItems = computed(() => TABS.map(tab => ({ key: tab.key, label: t(tab.titleKey) })));

const PARAM_GROUPS = {
  basic: [
    { key: 'path', type: 'text', picker: true, lockWhenRunning: true },
    { key: 'host.hostname', type: 'text' },
    { key: 'host.domainname', type: 'text' },
    { key: 'host.hostuuid', type: 'text', descKey: 'jails.descHostuuid' },
    { key: 'allow.set_hostname', type: 'bool', descKey: 'jails.descAllowSetHostname' },
    { key: '__autoStart', type: 'bool', descKey: 'jails.descAutoStart' },
  ],
  network: [
    { key: 'interface', type: 'text', descKey: 'jails.descInterface', lockWhenRunning: true },
    { key: 'ip4', type: 'select', lockWhenRunning: true, options: [
      { value: '', label: '—' },
      { value: 'inherit', label: 'inherit' },
      { value: 'disable', label: 'disable' },
    ]},
    { key: 'ip4.addr', type: 'text', descKey: 'jails.descIpAddr', lockWhenRunning: true, ph: '192.168.1.10 or bge1|192.168.1.10' },
    { key: 'ip6', type: 'select', lockWhenRunning: true, options: [
      { value: '', label: '—' },
      { value: 'inherit', label: 'inherit' },
      { value: 'disable', label: 'disable' },
    ]},
    { key: 'ip6.addr', type: 'text', lockWhenRunning: true, ph: '2001:db8::1' },
    { key: 'vnet', type: 'bool', descKey: 'jails.descVnet', lockWhenRunning: true },
    { key: 'allow.raw_sockets', type: 'bool', descKey: 'jails.descAllowRawSockets' },
    { key: 'allow.socket_af', type: 'bool', descKey: 'jails.descAllowSocketAf' },
    { key: 'allow.reserved_ports', type: 'bool' },
  ],
  exec: [
    { key: 'exec.start', type: 'text', descKey: 'jails.descExecStart', lockWhenRunning: true },
    { key: 'exec.stop', type: 'text', descKey: 'jails.descExecStop', lockWhenRunning: true },
    { key: 'exec.clean', type: 'bool', descKey: 'jails.descExecClean', lockWhenRunning: true },
    { key: 'exec.jail_user', type: 'text', lockWhenRunning: true },
    { key: 'exec.system_user', type: 'text', lockWhenRunning: true },
    { key: 'exec.prestart', type: 'text' },
    { key: 'exec.poststop', type: 'text' },
    { key: 'exec.timeout', type: 'text', descKey: 'jails.descExecTimeout', lockWhenRunning: true },
    { key: 'exec.consolelog', type: 'text' },
  ],
  mount: [
    { key: 'mount.fstab', type: 'text', picker: true, descKey: 'jails.descMountFstab', lockWhenRunning: true },
    { key: 'mount.devfs', type: 'bool', descKey: 'jails.descMountDevfs', lockWhenRunning: true },
    { key: 'mount.fdescfs', type: 'bool', lockWhenRunning: true },
    { key: 'mount.procfs', type: 'bool', lockWhenRunning: true },
    { key: 'allow.mount', type: 'bool' },
    { key: 'allow.mount.devfs', type: 'bool' },
    { key: 'allow.mount.fdescfs', type: 'bool' },
    { key: 'allow.mount.procfs', type: 'bool' },
    { key: 'allow.mount.nullfs', type: 'bool' },
    { key: 'allow.mount.zfs', type: 'bool' },
    { key: 'allow.quotas', type: 'bool' },
  ],
  security: [
    { key: 'securelevel', type: 'text', descKey: 'jails.descSecurelevel', lockWhenRunning: true },
    { key: 'enforce_statfs', type: 'select', descKey: 'jails.descEnforceStatfs', lockWhenRunning: true, options: [
      { value: '', label: '—' },
      { value: '0', label: '0' },
      { value: '1', label: '1' },
      { value: '2', label: '2' },
    ]},
    { key: 'devfs_ruleset', type: 'text', descKey: 'jails.descDevfsRuleset', lockWhenRunning: true },
    { key: 'children.max', type: 'text', descKey: 'jails.descChildrenMax', lockWhenRunning: true },
    { key: 'allow.suser', type: 'bool', descKey: 'jails.descAllowSuser' },
    { key: 'allow.chflags', type: 'bool', descKey: 'jails.descAllowChflags' },
    { key: 'allow.sysvipc', type: 'bool', descKey: 'jails.descAllowSysvipc' },
    { key: 'allow.unprivileged_proc_debug', type: 'bool' },
  ],
  misc: [
    { key: 'persist', type: 'bool', descKey: 'jails.descPersist', lockWhenRunning: true },
    { key: 'parent', type: 'text', descKey: 'jails.descParent', lockWhenRunning: true },
    { key: 'allow.read_conf', type: 'bool' },
    { key: 'allow.write_conf', type: 'bool' },
  ],
};

const definedKeys = computed(() => {
  const keys = new Set();
  for (const group of Object.values(PARAM_GROUPS)) {
    for (const p of group) keys.add(p.key);
  }
  return keys;
});

const extraParams = computed(() => {
  if (!jail.value) return [];
  const all = Object.keys(jail.value.params || {});
  return all.filter(k => !definedKeys.value.has(k) && !JAIL_READONLY.has(k)).sort();
});

const isRunning = computed(() => jail.value?.jid > 0);

function boolVal(key) {
  const v = form.value[key];
  return v === 'true' || v === '1';
}

function isLocked(p) {
  return isRunning.value && p.lockWhenRunning;
}

async function onSubmit() {
  submitting.value = true;
  try {
    const params = { ...form.value };
    delete params.__autoStart;
    await api.put(`/api/jails/${encodeURIComponent(name)}`, {
      params,
      auto_start: autoStart.value,
    });
    toast.toast(t('jails.jailUpdated'));
    router.push(`/jails/detail/${name}`);
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    submitting.value = false;
  }
}

onMounted(async () => {
  try {
    jail.value = await api.get(`/api/jails/${encodeURIComponent(name)}`);
    form.value = { ...(jail.value.params || {}) };
    autoStart.value = !!jail.value.auto_start;
    await loadFstab();
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton :href="`#/jails/detail/${name}`" />
      <h1>{{ t('jails.editTitle') }} — {{ name }}</h1>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!jail" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <div v-if="isRunning" class="card" style="border-color: var(--warn); border-style: dashed;">
      <i class="fa-solid fa-triangle-exclamation" style="color: var(--warn);"></i>
      <span style="margin-left: 8px;">{{ t('jails.editRunningWarn') }}</span>
    </div>

    <form @submit.prevent="onSubmit">
      <SectionCard v-model="activeTab" :tabs="tabItems">
        <template #default="{ active }">
        <template v-for="tab in TABS" :key="tab.key">
        <div v-if="active === tab.key">
        <div v-for="p in PARAM_GROUPS[tab.key]" :key="p.key" class="form-row">
          <label class="form-row-label">
            <span class="mono">{{ p.key === '__autoStart' ? t('jails.autoStart') : p.key }}</span>
          </label>

          <div>
            <label v-if="p.type === 'bool'" class="checkbox-label" :class="{ 'lock-disabled': isLocked(p) }">
              <input
                v-if="p.key === '__autoStart'"
                type="checkbox"
                v-model="autoStart"
              />
              <input
                v-else
                type="checkbox"
                :checked="boolVal(p.key)"
                :disabled="isLocked(p)"
                @change="form[p.key] = $event.target.checked ? 'true' : ''"
              />
              <span class="param-desc-inline" v-if="p.descKey">{{ t(p.descKey) }}</span>
            </label>

            <div v-else>
              <select v-if="p.type === 'select'" v-model="form[p.key]" :disabled="isLocked(p)">
                <option v-for="opt in p.options" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
              </select>

              <div v-else-if="p.picker" class="input-with-btn">
                <input type="text" v-model="form[p.key]" :placeholder="p.ph || ''" :disabled="isLocked(p)" />
                <button type="button" class="btn-secondary btn-sm" :disabled="isLocked(p)" @click="openPicker(p.key)">
                  <i class="fa-solid fa-folder-open"></i>
                </button>
                <button v-if="p.key === 'mount.fstab'" type="button" class="btn-secondary btn-sm" @click="showFstabModal = true">
                  <i class="fa-solid fa-list"></i> {{ t('jails.fstabManage') }}
                </button>
              </div>

              <input v-else type="text" v-model="form[p.key]" :placeholder="p.ph || ''" :disabled="isLocked(p)" />

              <p v-if="p.descKey" class="param-desc">{{ t(p.descKey) }}</p>
            </div>
          </div>
        </div>
        </div>
        </template>
        </template>
      </SectionCard>

      <!-- Extra params not in any group -->
      <div v-if="extraParams.length" class="card">
        <div class="card-title">{{ t('jails.editRawParams') }}</div>
        <div v-for="key in extraParams" :key="key" class="form-row">
          <label><span class="mono">{{ key }}</span></label>
          <div>
            <input type="text" v-model="form[key]" />
          </div>
        </div>
      </div>

      <div class="form-actions-bar">
        <a :href="`#/jails/detail/${name}`" class="btn btn-secondary">{{ t('common.cancel') }}</a>
        <button type="submit" :disabled="submitting">{{ t('common.save') }}</button>
      </div>
    </form>

    <FilePicker
      v-if="pickerTarget"
      :mode="pickerTarget === 'mount.fstab' ? 'file' : 'dir'"
      :initial-path="pickerTarget === '__fstab' ? (fstabEditing?.fs_spec || '/') : (form[pickerTarget] || '/')"
      @select="onPickerSelect"
      @close="pickerTarget = null"
    />

    <!-- Fstab management modal -->
    <div v-if="showFstabModal" class="modal-overlay" @click.self="showFstabModal = false">
      <div class="modal" style="max-width:640px;">
        <h3>{{ t('jails.fstabManage') }}</h3>
        <div v-if="!fstabEntries.length && !fstabEditing" class="text-dim" style="margin-bottom:12px;">{{ t('jails.fstabEmpty') }}</div>
        <table v-if="fstabEntries.length && !fstabEditing" class="fstab-table" style="margin-bottom:12px;">
          <thead><tr>
            <th>{{ t('jails.fstabSource') }}</th>
            <th>{{ t('jails.fstabTarget') }}</th>
            <th>{{ t('jails.fstabType') }}</th>
            <th>{{ t('jails.fstabMode') }}</th>
            <th>{{ t('common.actions') }}</th>
          </tr></thead>
          <tbody>
            <tr v-for="(e, idx) in fstabEntries" :key="idx">
              <td class="mono">{{ e.fs_spec }}</td>
              <td class="mono">{{ e.fs_file }}</td>
              <td class="mono">{{ e.fs_vfstype }}</td>
              <td class="mono">{{ e.fs_mntops === 'rw' ? t('jails.fstabReadWrite') : t('jails.fstabReadOnly') }}</td>
              <td>
                <div class="btn-group">
                  <button type="button" class="btn-secondary btn-sm" @click="fstabEdit(idx)">{{ t('common.edit') }}</button>
                  <button type="button" class="btn-danger btn-sm" @click="fstabDelete(idx)">{{ t('common.delete') }}</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>

        <!-- Entry editor form (inline within modal) -->
        <template v-if="fstabEditing">
          <div class="field">
            <label>{{ t('jails.fstabSource') }} <span style="color:var(--danger)">*</span></label>
            <div class="input-with-btn">
              <input type="text" v-model="fstabEditing.fs_spec" placeholder="/usr/jails/sharedfs" />
              <button type="button" class="btn-secondary btn-sm" @click="openPicker('__fstab')"><i class="fa-solid fa-folder-open"></i></button>
            </div>
          </div>
          <div class="field">
            <label>{{ t('jails.fstabTarget') }} <span style="color:var(--danger)">*</span></label>
            <input type="text" v-model="fstabEditing.fs_file" placeholder="/sharedfs" />
          </div>
          <div class="field">
            <label class="checkbox-label">
              <input type="checkbox" :checked="fstabEditing.fs_mntops === 'ro'" @change="fstabEditing.fs_mntops = $event.target.checked ? 'ro' : 'rw'" />
              <span>{{ t('jails.fstabReadOnly') }}</span>
            </label>
          </div>
        </template>

        <div class="modal-actions">
          <template v-if="fstabEditing">
            <button type="button" class="btn-secondary" @click="fstabEditing = null">{{ t('common.cancel') }}</button>
            <button type="button" @click="fstabSaveEntry">{{ t('common.save') }}</button>
          </template>
          <template v-else>
            <button type="button" class="btn-secondary" @click="showFstabModal = false">{{ t('common.close') }}</button>
            <button type="button" @click="fstabAdd"><i class="fa-solid fa-plus"></i> {{ t('jails.fstabAdd') }}</button>
          </template>
        </div>
      </div>
    </div>
  </template>
</template>

<style scoped>
.form-row-label {
  padding-top: 8px;
}
.param-desc {
  font-size: 12px;
  color: var(--text-dim);
  margin: 4px 0 0 0;
}
.param-desc-inline {
  font-size: 12px;
  color: var(--text-dim);
}
.checkbox-label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  padding-top: 8px;
}
.lock-disabled {
  opacity: 0.4;
  pointer-events: none;
}
input:disabled, select:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.fstab-section {
  margin-top: 20px;
  border-top: 1px solid var(--border);
  padding-top: 16px;
}
.fstab-table {
  width: 100%;
  border-collapse: collapse;
}
.fstab-table th, .fstab-table td {
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
  text-align: left;
}
.fstab-table th {
  color: var(--text-dim);
  font-weight: normal;
}
</style>
