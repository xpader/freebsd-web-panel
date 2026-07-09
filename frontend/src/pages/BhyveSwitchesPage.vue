<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';

const { t } = useI18n();

const switches = ref([]);
const loading = ref(true);
const error = ref('');

async function load() {
  loading.value = true;
  error.value = '';
  try {
    switches.value = await api.get('/api/bhyve/switches');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.tabSwitches') }}</h1>
    <p>{{ t('bhyve.switchSubtitle') }}</p>
  </div>

  <div class="toolbar">
    <span class="text-dim">{{ t('bhyve.switchCount', { n: switches.length }) }}</span>
    <button @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('common.type') }}</th>
        <th>{{ t('bhyve.interface') }}</th>
        <th>{{ t('bhyve.address') }}</th>
        <th>{{ t('bhyve.private') }}</th>
        <th>MTU</th>
        <th>VLAN</th>
        <th>{{ t('bhyve.ports') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="8" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="8" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!switches.length"><td colspan="8" class="empty">{{ t('bhyve.noSwitches') }}</td></tr>
        <tr v-for="sw in switches" :key="sw.name">
          <td class="mono"><strong>{{ sw.name }}</strong></td>
          <td>{{ sw.type }}</td>
          <td class="mono">{{ sw.iface }}</td>
          <td class="mono">{{ sw.address || '—' }}</td>
          <td>
            <span :class="['badge', sw.private ? 'badge-warn' : 'badge-dim']">{{ sw.private ? t('common.yes') : t('common.no') }}</span>
          </td>
          <td class="mono">{{ sw.mtu || '—' }}</td>
          <td class="mono">{{ sw.vlan || '—' }}</td>
          <td class="mono">{{ sw.ports.length ? sw.ports.join(', ') : '—' }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
