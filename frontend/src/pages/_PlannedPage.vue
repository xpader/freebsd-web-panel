<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';

const props = defineProps({
  moduleKey: { type: String, required: true },
  labelKey: { type: String, required: true },
});

const { t } = useI18n();
const status = ref(null);
const error = ref(null);

onMounted(async () => {
  try {
    status.value = await api.get(`/api/${props.moduleKey}`);
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t(labelKey) }}</h1>
    <p>{{ t(`planned.${moduleKey}.desc`) }}</p>
  </div>
  <div class="card">
    <div v-if="!status && !error" class="empty">
      <span class="spinner"></span> {{ t('planned.checking') }}
    </div>
    <div v-else-if="error">
      <div class="card-title">{{ t('common.moduleStatus') }}</div>
      <p class="text-dim">{{ t('planned.getStatusFailed', { msg: error }) }}</p>
    </div>
    <div v-else>
      <div class="card-title">{{ t('common.moduleStatus') }}</div>
      <div class="flex">
        <span :class="['badge', status.status === 'planned' ? 'badge-warn' : 'badge-success']">{{ status.status }}</span>
        <span class="text-dim">{{ status.message }}</span>
      </div>
    </div>
  </div>
  <div class="card">
    <div class="card-title">{{ t('planned.plan') }}</div>
    <p class="text-dim">{{ t(`planned.${moduleKey}.detail`) }}</p>
    <p class="text-dim mt-8">{{ t('planned.skeletonNote') }}</p>
  </div>
</template>
