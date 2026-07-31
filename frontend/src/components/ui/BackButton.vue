<script setup>
import { useI18n } from 'vue-i18n';

defineProps({
  href: { type: String, required: true },
});

const { t } = useI18n();
</script>

<template>
  <a :href="href" class="back-btn" :title="t('common.back')">
    <i class="fa-solid fa-arrow-left"></i>
  </a>
</template>

<style scoped>
.back-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: var(--radius);
  background: var(--bg-elev2);
  border: 1px solid var(--border);
  color: var(--text);
  font-size: 18px;
  cursor: pointer;
  transition: background 0.15s, transform 0.08s ease;
}
.back-btn:hover { background: var(--border); }
.back-btn:active { transform: scale(0.96); }
</style>
