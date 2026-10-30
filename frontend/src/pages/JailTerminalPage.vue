<script setup>
import { useI18n } from 'vue-i18n';
import { useRoute } from 'vue-router';
import TermView from '../components/ui/TermView.vue';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;
</script>

<template>
  <TermView :query="{ jail: name }" :back="`#/jails/detail/${name}`">
    <template #title>
      <h1>{{ t('term.jailTitle') }}: {{ name }}</h1>
    </template>
  </TermView>
</template>
