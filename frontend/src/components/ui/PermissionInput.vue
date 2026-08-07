<script setup>
// Reusable octal permission editor.
//
// v-model is an octal *string* (e.g. '0644', '4755') — the universal
// serialized form shared by the file-manager chmod dialog and Samba mask
// fields. Checkboxes are the primary input; the octal field is editable for
// power users and syncs both ways.
//
// `special` hides the setuid/setgid/sticky bits and constrains the value to
// the lower 3 octal digits (0o777) — used for Samba create/directory masks
// where special bits are meaningless.

import { ref, computed, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { permStringFull, octStr } from '../../lib/format.js';

const props = defineProps({
  modelValue: { type: String, default: '0644' },
  special: { type: Boolean, default: true },
});
const emit = defineEmits(['update:modelValue']);
const { t } = useI18n();

const mask = computed(() => (props.special ? 0o7777 : 0o0777));

const numericMode = computed(() => {
  const n = parseInt(props.modelValue, 8);
  return (Number.isNaN(n) ? 0 : n) & mask.value;
});

function emitNumber(n) {
  emit('update:modelValue', octStr(n & mask.value));
}

function setBit(bit, checked) {
  const cur = numericMode.value;
  emitNumber(checked ? (cur | bit) : (cur & ~bit));
}

const rows = computed(() => [
  { whoLabel: t('common.owner'), r: 0o400, w: 0o200, x: 0o100, s: 0o4000, specialLabel: 'setuid' },
  { whoLabel: t('common.group'), r: 0o040, w: 0o020, x: 0o010, s: 0o2000, specialLabel: 'setgid' },
  { whoLabel: t('common.other'), r: 0o004, w: 0o002, x: 0o001, s: 0o1000, specialLabel: 'sticky' },
]);

// Editable octal text. Keep the user's raw input while focused (so transient
// values like "7" don't snap to "0007"); normalize on blur and sync from the
// parent whenever the field is not being edited.
const octalText = ref(props.modelValue);
let octalFocused = false;
watch(() => props.modelValue, (v) => { if (!octalFocused) octalText.value = v; });

function onOctalFocus() { octalFocused = true; }
function onOctalInput(e) {
  octalText.value = e.target.value;
  const cleaned = e.target.value.replace(/[^0-7]/g, '');
  emitNumber(cleaned ? parseInt(cleaned, 8) : 0);
}
function onOctalBlur() {
  octalFocused = false;
  octalText.value = octStr(numericMode.value);
}
</script>

<template>
  <div class="perm-input">
    <div class="perm-grid">
      <div v-for="row in rows" :key="row.whoLabel" class="perm-row">
        <span class="perm-who">{{ row.whoLabel }}</span>
        <label class="perm-check"><input type="checkbox" :checked="numericMode & row.r" @change="setBit(row.r, $event.target.checked)" />{{ t('common.read') }}</label>
        <label class="perm-check"><input type="checkbox" :checked="numericMode & row.w" @change="setBit(row.w, $event.target.checked)" />{{ t('common.write') }}</label>
        <label class="perm-check"><input type="checkbox" :checked="numericMode & row.x" @change="setBit(row.x, $event.target.checked)" />{{ t('common.execute') }}</label>
        <label v-if="special" class="perm-check"><input type="checkbox" :checked="numericMode & row.s" @change="setBit(row.s, $event.target.checked)" />{{ row.specialLabel }}</label>
      </div>
    </div>
    <div class="perm-preview">
      <span class="text-dim">{{ t('common.octalMode') }}:</span>
      <input class="perm-octal mono" :value="octalText" size="4" maxlength="4"
        @focus="onOctalFocus" @input="onOctalInput" @blur="onOctalBlur" />
      <span class="mono perm-str">{{ permStringFull(numericMode) }}</span>
    </div>
  </div>
</template>

<style scoped>
.perm-grid { display: flex; flex-direction: column; gap: 10px; margin: 8px 0 16px; }
.perm-row { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
.perm-who { width: 56px; font-size: 13px; color: var(--text-dim); flex-shrink: 0; }
.perm-check { display: flex; align-items: center; gap: 5px; font-size: 13px; cursor: pointer; }
.perm-check input { width: auto; margin: 0; }
.perm-preview { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 10px 12px; background: var(--bg-elev2); border-radius: var(--radius); margin-bottom: 12px; flex-wrap: wrap; }
.perm-octal { width: 64px; padding: 4px 8px; font-size: 13px; }
.perm-str { color: var(--text-dim); }
</style>
