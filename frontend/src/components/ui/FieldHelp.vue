<script setup>
import { ref } from 'vue';

defineProps({
  text: { type: String, default: '' },
});

const show = ref(false);
</script>

<template>
  <button v-if="text" type="button" class="field-help" @click="show = !show" @blur="show = false">
    <i class="fa-solid fa-circle-info"></i>
    <span v-if="show" class="field-tooltip" @mousedown.prevent>{{ text }}</span>
  </button>
</template>
