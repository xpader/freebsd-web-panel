<script setup>
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
</script>

<template>
  <div class="main">
    <div class="empty">
      <h1>404</h1>
      <p>{{ t('common.notFound') }}</p>
    </div>
  </div>
</template>
