<script setup>
import { useUiStore } from '../../stores/ui.js';

const ui = useUiStore();
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
