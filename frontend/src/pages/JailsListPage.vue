<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const jailTab = ref('all');
const allJails = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const needsInit = ref(false);
const initializing = ref(false);
const pendingActions = ref(new Set());

function jailState(name, running) {
  if (pendingActions.value.has(`${name}:start`)) return 'starting';
  if (pendingActions.value.has(`${name}:stop`)) return 'stopping';
  return running ? 'running' : 'stopped';
}

function formatIpStr(ip4, ip6) {
  const parts = [];
  if (ip4) parts.push(ip4);
  if (ip6) parts.push(ip6);
  if (!parts.length) return '';
  return parts.join(', ');
}

function stateBadge(state) {
  if (state === 'running') return { cls: 'badge-success', text: t('jails.running') };
  if (state === 'starting') return { cls: 'badge-warn', text: t('jails.starting') };
  if (state === 'stopping') return { cls: 'badge-warn', text: t('jails.stopping') };
  return { cls: 'badge-dim', text: t('jails.stopped') };
}

async function load() {
  if (!allJails.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    const url = jailTab.value === 'running' ? '/api/jails?running=true' : '/api/jails';
    allJails.value = await api.get(url);
    needsInit.value = false;
  } catch (err) {
    if (err.status === 428) {
      needsInit.value = true;
    } else {
      error.value = err.message || '';
    }
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function jailAction(name, action) {
  pendingActions.value.add(`${name}:${action}`);
  try {
    await api.post(`/api/jails/${encodeURIComponent(name)}/${action}`);
    toast.toast(t('jails.actionDone', { name, action: t('jails.' + action) }));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    pendingActions.value.delete(`${name}:${action}`);
    await load();
  }
}

async function jailDelete(name) {
  const result = await confirm(t('jails.deleteJail'), t('jails.deleteConfirm', { name }), [
    { key: 'removeFiles', label: t('jails.deleteFiles'), checked: false },
  ]);
  if (!result || !result.confirmed) return;
  try {
    const qs = result.removeFiles ? '?remove_files=true' : '';
    await api.del(`/api/jails/${encodeURIComponent(name)}${qs}`);
    toast.toast(t('jails.deleted'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doInit() {
  initializing.value = true;
  try {
    await api.post('/api/jails/init');
    toast.toast(t('jails.initDone'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    initializing.value = false;
  }
}

onMounted(load);
</script>

<template>
  <!-- Initialization prompt -->
  <template v-if="needsInit">
    <div class="page-header">
      <h1>{{ t('jails.title') }}</h1>
      <p>{{ t('jails.subtitle') }}</p>
    </div>
    <div class="card init-card">
      <div class="init-icon"><i class="fa-solid fa-cube"></i></div>
      <h2>{{ t('jails.initTitle') }}</h2>
      <p class="text-dim init-desc">{{ t('jails.initDesc') }}</p>
      <div class="init-files">
        <div class="init-file">
          <i class="fa-solid fa-file-lines"></i>
          <code>/etc/jail.conf</code>
          <span class="text-dim">{{ t('jails.initFileJailConf') }}</span>
        </div>
        <div class="init-file">
          <i class="fa-solid fa-shield-halved"></i>
          <code>/etc/devfs.rules</code>
          <span class="text-dim">{{ t('jails.initFileDevfs') }}</span>
        </div>
      </div>
      <button class="btn-primary init-btn" @click="doInit" :disabled="initializing">
        <i :class="['fa-solid fa-rocket', { 'fa-spin': initializing }]"></i>
        {{ t('jails.initButton') }}
      </button>
    </div>
  </template>

  <!-- Normal list view -->
  <template v-else>
  <div class="page-header">
    <h1>{{ t('jails.title') }}</h1>
    <p>{{ t('jails.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <div class="filter-group">
      <button :class="['filter-btn', { active: jailTab === 'all' }]" @click="jailTab = 'all'; load()">{{ t('common.all') }}</button>
      <button :class="['filter-btn', { active: jailTab === 'running' }]" @click="jailTab = 'running'; load()">{{ t('jails.running') }}</button>
    </div>
    <span class="text-dim">{{ t('jails.count', { n: allJails.length }) }}</span>
    <div class="flex">
      <button @click="router.push('/jails/create')"><i class="fa-solid fa-plus"></i> {{ t('jails.create') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>JID</th><th>{{ t('common.name') }}</th><th>{{ t('common.description') }}</th>
        <th>IP</th><th>{{ t('common.status') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!allJails.length"><td colspan="6" class="empty">{{ t('jails.noJails') }}</td></tr>
        <tr v-for="j in allJails" :key="j.name" class="row-clickable" @click="router.push(`/jails/detail/${j.name}`)">
          <td class="mono">{{ j.jid > 0 ? j.jid : '—' }}</td>
          <td class="mono"><strong>{{ j.name }}</strong></td>
          <td>{{ j.description || '—' }}</td>
          <td class="mono">
            <span v-if="formatIpStr(j.ip4_addr, j.ip6_addr)" class="badge badge-dim">{{ formatIpStr(j.ip4_addr, j.ip6_addr) }}</span>
            <span v-else class="text-dim">—</span>
          </td>
          <td><span :class="['badge', stateBadge(jailState(j.name, jailTab === 'running' ? true : j.jid > 0)).cls]">{{ stateBadge(jailState(j.name, jailTab === 'running' ? true : j.jid > 0)).text }}</span></td>
          <td>
            <div class="btn-group" @click.stop>
              <template v-if="['starting', 'stopping'].includes(jailState(j.name, j.jid > 0))">
                <button class="btn-secondary btn-sm" disabled>{{ t('jails.start') }}</button>
                <button class="btn-secondary btn-sm" disabled>{{ t('jails.stop') }}</button>
              </template>
              <template v-else>
                <button class="btn-secondary btn-sm" :disabled="jailState(j.name, j.jid > 0) === 'running'" @click="jailAction(j.name, 'start')">{{ t('jails.start') }}</button>
                <button class="btn-secondary btn-sm" :disabled="jailState(j.name, j.jid > 0) !== 'running'" @click="jailAction(j.name, 'stop')">{{ t('jails.stop') }}</button>
              </template>
              <a v-if="j.jid > 0" :href="`#/jails/terminal/${j.name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-terminal"></i> {{ t('term.openTerminal') }}</a>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
  </template>
</template>

<style scoped>
.init-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 48px 24px;
  gap: 12px;
}
.init-icon {
  font-size: 48px;
  color: var(--accent);
  margin-bottom: 8px;
}
.init-card h2 {
  margin: 0;
}
.init-desc {
  max-width: 500px;
}
.init-files {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 16px 0;
  width: 100%;
  max-width: 480px;
}
.init-file {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 16px;
  background: var(--bg-elev);
  border-radius: var(--radius);
}
.init-file .text-dim {
  margin-left: auto;
  font-size: 13px;
}
.init-btn {
  margin-top: 8px;
}
</style>
