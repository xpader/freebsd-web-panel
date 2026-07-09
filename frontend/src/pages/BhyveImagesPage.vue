<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';

const { t } = useI18n();

const images = ref([]);
const loading = ref(true);
const error = ref('');

async function load() {
  loading.value = true;
  error.value = '';
  try {
    images.value = await api.get('/api/bhyve/images');
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
    <h1>{{ t('bhyve.tabImages') }}</h1>
    <p>{{ t('bhyve.imageSubtitle') }}</p>
  </div>

  <div class="toolbar">
    <span class="text-dim">{{ t('bhyve.imageCount', { n: images.length }) }}</span>
    <button @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>UUID</th>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('common.createdAt') }}</th>
        <th>{{ t('common.description') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="4" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="4" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!images.length"><td colspan="4" class="empty">{{ t('bhyve.noImages') }}</td></tr>
        <tr v-for="img in images" :key="img.uuid">
          <td class="mono">{{ img.uuid }}</td>
          <td class="mono"><strong>{{ img.name }}</strong></td>
          <td class="mono">{{ img.created }}</td>
          <td>{{ img.description || '—' }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
