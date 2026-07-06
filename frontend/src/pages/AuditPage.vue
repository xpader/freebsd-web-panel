<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtTime } from '../lib/format.js';

const { t } = useI18n();
const entries = ref(null);
const error = ref('');

function methodBadge(m) {
  if (m === 'GET') return 'badge-dim';
  if (m === 'DELETE') return 'badge-danger';
  return 'badge-warn';
}
function statusBadge(s) {
  if (s >= 200 && s < 300) return 'badge-success';
  if (s >= 400 && s < 500) return 'badge-warn';
  if (s >= 500) return 'badge-danger';
  return 'badge-dim';
}

onMounted(async () => {
  try {
    const res = await api.get('/api/audit?limit=200');
    entries.value = res.entries || [];
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('audit.title') }}</h1>
    <p>{{ t('audit.subtitle') }}</p>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('audit.time') }}</th>
        <th>{{ t('common.user') }}</th>
        <th>{{ t('audit.method') }}</th>
        <th>{{ t('audit.path') }}</th>
        <th>{{ t('common.status') }}</th>
        <th>{{ t('audit.detail') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="!entries"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!entries.length"><td colspan="6" class="empty">{{ t('audit.noLogs') }}</td></tr>
        <tr v-for="(e, i) in entries" :key="i">
          <td class="mono text-dim">{{ fmtTime(e.ts) }}</td>
          <td>{{ e.user || '—' }}</td>
          <td><span :class="['badge', methodBadge(e.method)]">{{ e.method }}</span></td>
          <td class="mono">{{ e.path }}</td>
          <td><span :class="['badge', statusBadge(e.status)]">{{ e.status }}</span></td>
          <td class="text-dim">{{ e.detail || '' }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
