<script setup>
import { useI18n } from 'vue-i18n';
import { useRoute } from 'vue-router';
import TermView from '../components/ui/TermView.vue';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;
</script>

<template>
  <TermView :query="{ vm: name }" :back="`#/bhyve/detail/${name}`">
    <template #title>
      <h1>{{ t('common.console') }}: {{ name }}</h1>
    </template>
  </TermView>
</template>
