<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter, useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import FilePicker from '../components/ui/FilePicker.vue';
import Tabs from '../components/ui/Tabs.vue';

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
  form.value[pickerTarget.value] = path;
  pickerTarget.value = null;
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
  { key: 'permissions', titleKey: 'jails.permissions' },
  { key: 'misc', titleKey: 'jails.editMisc' },
];
const tabItems = computed(() => TABS.map(tab => ({ key: tab.key, label: t(tab.titleKey) })));

const PARAM_GROUPS = {
  basic: [
    { key: 'path', type: 'text', picker: true, lockWhenRunning: true },
    { key: 'host.hostname', type: 'text' },
    { key: 'host.domainname', type: 'text' },
    { key: 'host.hostuuid', type: 'text', descKey: 'jails.descHostuuid' },
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
  ],
  permissions: [
    { key: 'allow.set_hostname', type: 'bool', descKey: 'jails.descAllowSetHostname' },
    { key: 'allow.sysvipc', type: 'bool', descKey: 'jails.descAllowSysvipc' },
    { key: 'allow.raw_sockets', type: 'bool', descKey: 'jails.descAllowRawSockets' },
    { key: 'allow.chflags', type: 'bool', descKey: 'jails.descAllowChflags' },
    { key: 'allow.socket_af', type: 'bool', descKey: 'jails.descAllowSocketAf' },
    { key: 'allow.mount', type: 'bool' },
    { key: 'allow.mount.devfs', type: 'bool' },
    { key: 'allow.mount.fdescfs', type: 'bool' },
    { key: 'allow.mount.procfs', type: 'bool' },
    { key: 'allow.mount.nullfs', type: 'bool' },
    { key: 'allow.mount.zfs', type: 'bool' },
    { key: 'allow.quotas', type: 'bool' },
    { key: 'allow.read_conf', type: 'bool' },
    { key: 'allow.write_conf', type: 'bool' },
    { key: 'allow.suser', type: 'bool', descKey: 'jails.descAllowSuser' },
    { key: 'allow.reserved_ports', type: 'bool' },
    { key: 'allow.unprivileged_proc_debug', type: 'bool' },
  ],
  misc: [
    { key: 'persist', type: 'bool', descKey: 'jails.descPersist', lockWhenRunning: true },
    { key: 'parent', type: 'text', descKey: 'jails.descParent', lockWhenRunning: true },
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
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <a :href="`#/jails/detail/${name}`" class="btn-secondary btn-sm">{{ t('common.navBack') }}</a>
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
      <Tabs v-model="activeTab" :tabs="tabItems">
        <div v-for="p in PARAM_GROUPS[activeTab]" :key="p.key" class="form-row">
          <label>
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
              <span v-if="p.key !== '__autoStart'"></span>
            </label>

            <select v-else-if="p.type === 'select'" v-model="form[p.key]" :disabled="isLocked(p)">
              <option v-for="opt in p.options" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
            </select>

            <div v-else-if="p.picker" class="input-with-btn">
              <input type="text" v-model="form[p.key]" :placeholder="p.ph || ''" :disabled="isLocked(p)" />
              <button type="button" class="btn-secondary btn-sm" :disabled="isLocked(p)" @click="openPicker(p.key)">
                <i class="fa-solid fa-folder-open"></i>
              </button>
            </div>

            <input v-else type="text" v-model="form[p.key]" :placeholder="p.ph || ''" :disabled="isLocked(p)" />

            <p v-if="p.descKey" class="param-desc">{{ t(p.descKey) }}</p>
          </div>
        </div>
      </Tabs>

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
      :initial-path="form[pickerTarget] || '/'"
      @select="onPickerSelect"
      @close="pickerTarget = null"
    />
  </template>
</template>

<style scoped>
.param-desc {
  font-size: 12px;
  color: var(--text-dim);
  margin: 4px 0 0 0;
}
.checkbox-label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
}
.lock-disabled {
  opacity: 0.4;
  pointer-events: none;
}
input:disabled, select:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
