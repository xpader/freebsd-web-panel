<script setup>
defineProps({
  modelValue: { type: String, default: '' },
  placeholder: { type: String, default: '' },
});
defineEmits(['update:modelValue']);
</script>

<template>
  <div class="search-input">
    <input
      type="text"
      class="filter-input"
      :value="modelValue"
      :placeholder="placeholder"
      @input="$emit('update:modelValue', $event.target.value)"
    />
    <button v-if="modelValue" class="search-clear" @click="$emit('update:modelValue', '')">
      <i class="fa-solid fa-xmark"></i>
    </button>
  </div>
</template>
