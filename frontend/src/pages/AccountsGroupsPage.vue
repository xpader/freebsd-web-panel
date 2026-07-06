<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';

const { t } = useI18n();
const allGroups = ref([]);
const loading = ref(true);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return allGroups.value;
  return allGroups.value.filter((g) =>
    g.name.toLowerCase().includes(q) || String(g.gid).includes(q) || g.members.some((m) => m.toLowerCase().includes(q))
  );
});

onMounted(async () => {
  try {
    allGroups.value = await api.get('/api/accounts/groups');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('accounts.groupsTitle') }}</h1>
    <p>{{ t('accounts.groupsSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <input type="text" v-model="filter" class="filter-input" :placeholder="t('accounts.filterGroup')" />
    <span class="text-dim">{{ t('accounts.groupCount', { n: filtered.length }) }}</span>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr><th>{{ t('auth.username') }}</th><th>{{ t('accounts.gid') }}</th><th>{{ t('accounts.members') }}</th></tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="3" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="3" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="3" class="empty">{{ t('accounts.noMatchGroup') }}</td></tr>
        <tr v-for="g in filtered" :key="g.gid">
          <td><strong>{{ g.name }}</strong></td>
          <td class="mono">{{ g.gid }}</td>
          <td>
            <template v-if="g.members.length">
              <span v-for="m in g.members" :key="m" class="badge badge-dim">{{ m }}</span>
            </template>
            <span v-else class="text-dim">—</span>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
