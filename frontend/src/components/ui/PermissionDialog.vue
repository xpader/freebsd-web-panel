<script setup>
// Octal permission editor as a modal overlay.
//
// Mirrors FilePicker.vue: a self-contained `.modal-overlay` (z-index 60 so it
// stacks above an active form modal). It wraps PermissionInput and returns the
// chosen octal string via `confirm`, or `close` on cancel.
//
// Props:
//   title   - dialog heading
//   value   - initial octal string (e.g. '0644', '4755')
//   special - show setuid/setgid/sticky bits (default true). False for Samba
//             masks where special bits are meaningless (value masked to 0o777).

import { ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import PermissionInput from './PermissionInput.vue';

const props = defineProps({
  title: { type: String, default: '' },
  value: { type: String, default: '0644' },
  special: { type: Boolean, default: true },
});
const emit = defineEmits(['confirm', 'close']);
const { t } = useI18n();

const mode = ref(props.value);
// Re-sync if reopened with a different value while still mounted.
watch(() => props.value, (v) => { mode.value = v; });
</script>

<template>
  <div class="modal-overlay perm-overlay" @click.self="emit('close')">
    <div class="modal">
      <h3>{{ title }}</h3>
      <PermissionInput v-model="mode" :special="special" />
      <div class="modal-actions">
        <button class="btn-secondary" @click="emit('close')">{{ t('common.cancel') }}</button>
        <button @click="emit('confirm', mode)">{{ t('common.ok') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.perm-overlay { z-index: 60; }
</style>
