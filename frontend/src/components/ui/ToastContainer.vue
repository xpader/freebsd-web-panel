<script setup>
import { ui } from '../../stores/ui.js';
</script>

<template>
  <div class="toast-wrap">
    <div
      v-for="t in ui.toasts"
      :key="t.id"
      :class="['toast', `toast-${t.type}`]"
      @click="ui.dismissToast(t.id)"
    >{{ t.message }}</div>
  </div>
</template>
