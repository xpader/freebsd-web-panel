<script setup>
defineProps({
  message: { type: String, required: true },
});
</script>

<template>
  <div class="card warn-banner">
    <i class="fa-solid fa-triangle-exclamation" style="color: var(--warn);"></i>
    <span style="margin-left: 8px;">{{ message }}</span>
  </div>
</template>

<style scoped>
.warn-banner {
  background: rgba(245, 158, 11, 0.08);
  border-left: 3px solid var(--warn);
  margin-bottom: 16px;
}
</style>
