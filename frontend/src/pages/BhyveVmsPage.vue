<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const vms = ref([]);
const loading = ref(true);
const vmFilter = ref('all');
const error = ref('');
const pendingActions = ref(new Set());
const bhyveStatus = ref(null);

const needsInit = computed(() => {
  if (!bhyveStatus.value) return false;
  const s = bhyveStatus.value;
  return !s.installed || !s.enabled || !s.initialized;
});

const initMessages = computed(() => {
  if (!bhyveStatus.value) return [];
  const s = bhyveStatus.value;
  const msgs = [];
  if (!s.installed) msgs.push(t('bhyve.initMissingPkg'));
  if (!s.enabled) msgs.push(t('bhyve.initMissingEnable'));
  if (!s.initialized) msgs.push(t('bhyve.initMissingDir'));
  return msgs;
});

function vmState(vm) {
  if (pendingActions.value.has(`${vm.name}:start`)) return 'starting';
  if (pendingActions.value.has(`${vm.name}:stop`)) return 'stopping';
  return vm.state;
}

function vmStateBadge(vm) {
  const st = vmState(vm);
  if (st === 'running' || st === 'starting') return { cls: 'badge-success', text: t('bhyve.stateRunning') };
  if (st === 'stopping') return { cls: 'badge-warn', text: t('bhyve.stopping') };
  if (st === 'locked') return { cls: 'badge-warn', text: t('bhyve.stateLocked') + (vm.locked_by ? ` (${vm.locked_by})` : '') };
  if (st === 'suspended') return { cls: 'badge-dim', text: t('bhyve.stateSuspended') };
  return { cls: 'badge-dim', text: t('bhyve.stateStopped') };
}

async function loadStatus() {
  try {
    bhyveStatus.value = await api.get('/api/bhyve/status');
  } catch { bhyveStatus.value = null; }
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    const url = vmFilter.value === 'running' ? '/api/bhyve/vms?running=true' : '/api/bhyve/vms';
    vms.value = await api.get(url);
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function vmAction(name, action) {
  pendingActions.value.add(`${name}:${action}`);
  try {
    await api.post(`/api/bhyve/vms/${encodeURIComponent(name)}/${action}`);
    toast.toast(action === 'start'
      ? t('bhyve.startedToast', { name })
      : t('bhyve.stoppedToast', { name }));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    pendingActions.value.delete(`${name}:${action}`);
    await new Promise((r) => setTimeout(r, 1000));
    await load();
  }
}

onMounted(async () => {
  await loadStatus();
  if (!needsInit.value) load();
  else loading.value = false;
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.tabVms') }}</h1>
    <p>{{ t('bhyve.subtitle') }}</p>
  </div>

  <!-- Not initialized banner -->
  <div v-if="needsInit" class="card" style="border:1px solid var(--warning);padding:24px;text-align:center;">
    <i class="fa-solid fa-triangle-exclamation" style="font-size:32px;color:var(--warning);"></i>
    <h3 style="margin:12px 0 8px;">{{ t('bhyve.initRequired') }}</h3>
    <p class="text-dim" style="margin-bottom:8px;">{{ t('bhyve.initRequiredDesc') }}</p>
    <ul v-if="initMessages.length" style="margin:8px auto 16px;display:inline-block;text-align:left;">
      <li v-for="(msg, i) in initMessages" :key="i" style="color:var(--warning);font-size:13px;">{{ msg }}</li>
    </ul>
    <div class="btn-group" style="justify-content:center;">
      <button @click="router.push('/bhyve/init')">
        <i class="fa-solid fa-rocket"></i> {{ t('bhyve.initGo') }}
      </button>
    </div>
  </div>

  <template v-else>
    <div class="toolbar">
      <div class="filter-group">
        <button :class="['filter-btn', { active: vmFilter === 'all' }]" @click="vmFilter = 'all'; load()">{{ t('common.all') }}</button>
        <button :class="['filter-btn', { active: vmFilter === 'running' }]" @click="vmFilter = 'running'; load()">{{ t('bhyve.stateRunning') }}</button>
      </div>
      <span class="text-dim">{{ t('bhyve.vmCount', { n: vms.length }) }}</span>
      <div class="flex btn-group">
        <button @click="router.push('/bhyve/create')"><i class="fa-solid fa-plus"></i> {{ t('bhyve.createVm') }}</button>
        <button @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
      </div>
    </div>

    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('bhyve.datastore') }}</th>
          <th>{{ t('bhyve.loader') }}</th>
          <th>CPU</th>
          <th>{{ t('bhyve.memory') }}</th>
          <th>VNC</th>
          <th>{{ t('bhyve.autoStart') }}</th>
          <th>{{ t('common.status') }}</th>
          <th>{{ t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="error"><td colspan="9" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
          <tr v-else-if="loading"><td colspan="9" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
          <tr v-else-if="!vms.length"><td colspan="9" class="empty">{{ t('bhyve.noVms') }}</td></tr>
          <tr v-for="vm in vms" :key="vm.name" class="row-clickable" @click="router.push(`/bhyve/detail/${vm.name}`)">
            <td class="mono"><strong>{{ vm.name }}</strong></td>
            <td>{{ vm.datastore }}</td>
            <td>{{ vm.loader }}</td>
            <td class="mono">{{ vm.cpu }}</td>
            <td class="mono">{{ vm.memory }}</td>
            <td class="mono">{{ vm.vnc || '—' }}</td>
            <td>
              <span v-if="vm.auto_start" class="badge badge-dim">{{ vm.auto_order != null ? `Yes [${vm.auto_order}]` : 'Yes' }}</span>
              <span v-else class="text-dim">No</span>
            </td>
            <td><span :class="['badge', vmStateBadge(vm).cls]">{{ vmStateBadge(vm).text }}</span></td>
            <td>
              <div class="btn-group" @click.stop>
                <button v-if="vmState(vm) !== 'running' && vmState(vm) !== 'starting'"
                  class="btn-secondary btn-sm"
                  :disabled="vmState(vm) === 'stopping' || vm.state === 'locked'"
                  @click="vmAction(vm.name, 'start')">
                  <i class="fa-solid fa-play"></i>
                </button>
                <button v-if="vmState(vm) === 'running' || vmState(vm) === 'stopping'"
                  class="btn-secondary btn-sm"
                  :disabled="vmState(vm) === 'stopping'"
                  @click="vmAction(vm.name, 'stop')">
                  <i class="fa-solid fa-stop"></i>
                </button>
                <button v-if="vmState(vm) === 'starting'" class="btn-secondary btn-sm" disabled>
                  <span class="spinner" style="width:12px;height:12px;"></span>
                </button>
                <a v-if="vm.state === 'running'" :href="`#/bhyve/console/${vm.name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-terminal"></i></a>
                <a v-if="vm.vnc && vm.state === 'running'" :href="`#/bhyve/vnc/${vm.name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-display"></i></a>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
