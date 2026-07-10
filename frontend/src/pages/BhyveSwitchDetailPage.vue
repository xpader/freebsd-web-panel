<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;
const detail = ref(null);
const error = ref('');

const fields = computed(() => {
  if (!detail.value?.fields) return [];
  return Object.entries(detail.value.fields).map(([key, value]) => ({ key, value }));
});

async function load() {
  error.value = '';
  try {
    detail.value = await api.get(`/api/bhyve/switches/${encodeURIComponent(name)}`);
  } catch (e) {
    error.value = e.message || '';
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/bhyve/switches" />
      <h1>{{ name }}</h1>
    </div>
    <button class="btn-secondary" @click="load" :disabled="!detail"><i class="fa-solid fa-rotate-right"></i> {{ t('common.refresh') }}</button>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!detail" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <div v-else class="card">
    <h3>{{ t('common.overview') }}</h3>
    <table>
      <thead><tr>
        <th>{{ t('common.key') }}</th>
        <th>{{ t('common.value') }}</th>
      </tr></thead>
      <tbody>
        <tr v-for="field in fields" :key="field.key">
          <td class="mono">{{ field.key }}</td>
          <td class="mono">{{ field.value }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
