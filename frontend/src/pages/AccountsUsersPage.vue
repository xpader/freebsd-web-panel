<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';

const { t } = useI18n();
const allUsers = ref([]);
const loading = ref(true);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return allUsers.value;
  return allUsers.value.filter((u) => u.name.toLowerCase().includes(q) || String(u.uid).includes(q));
});

onMounted(async () => {
  try {
    allUsers.value = await api.get('/api/accounts/users');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('accounts.usersTitle') }}</h1>
    <p>{{ t('accounts.usersSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <input type="text" v-model="filter" class="filter-input" :placeholder="t('accounts.filterUser')" />
    <span class="text-dim">{{ t('accounts.userCount', { n: filtered.length }) }}</span>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr><th>{{ t('auth.username') }}</th><th>{{ t('accounts.uid') }}</th><th>{{ t('accounts.group') }}</th><th>{{ t('common.description') }}</th><th>{{ t('accounts.home') }}</th><th>Shell</th></tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="6" class="empty">{{ t('accounts.noMatchUser') }}</td></tr>
        <tr v-for="u in filtered" :key="u.uid">
          <td><strong>{{ u.name }}</strong></td>
          <td class="mono">{{ u.uid }}</td>
          <td class="mono">{{ u.group_name || '—' }} <span class="text-dim">({{ u.gid }})</span></td>
          <td class="text-dim">{{ u.gecos || '—' }}</td>
          <td class="mono">{{ u.home }}</td>
          <td class="mono">{{ u.shell }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
