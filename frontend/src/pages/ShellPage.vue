<script setup>
import { useI18n } from 'vue-i18n';
import TermView from '../components/ui/TermView.vue';

const { t } = useI18n();
</script>

<template>
  <TermView>
    <template #title>
      <h1>{{ t('term.title') }}</h1>
    </template>
  </TermView>
</template>
