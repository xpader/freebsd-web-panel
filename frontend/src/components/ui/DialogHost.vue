<script setup>
import { ref, nextTick, watch } from 'vue';
import { useUiStore } from '../../stores/ui.js';
import { useI18n } from 'vue-i18n';

const ui = useUiStore();
const { t } = useI18n();

const firstInput = ref(null);

// Auto-focus first input when a form dialog opens.
watch(() => ui.dialog, (d) => {
  if (d && d.type === 'form') {
    nextTick(() => {
      if (firstInput.value) firstInput.value.focus();
    });
  }
});

function handleConfirm() {
  if (ui.dialog?.options?.length) {
    const result = { confirmed: true };
    for (const o of ui.dialog.options) {
      const cb = document.querySelector(`[data-opt="${o.key}"]`);
      result[o.key] = cb ? cb.checked : false;
    }
    ui.resolveDialog(result);
  } else {
    ui.resolveDialog(true);
  }
}

function handleFormSubmit(ev) {
  const formData = new FormData(ev.target);
  const result = {};
  for (const f of ui.dialog.fields) {
    result[f.key] = formData.get(f.key);
  }
  ui.resolveDialog(result);
}
</script>

<template>
  <div v-if="ui.dialog" class="modal-overlay">
    <!-- Confirm dialog -->
    <div v-if="ui.dialog.type === 'confirm'" class="modal">
      <h3>{{ ui.dialog.title }}</h3>
      <p class="text-dim">{{ ui.dialog.message }}</p>
      <label
        v-for="o in (ui.dialog.options || [])"
        :key="o.key"
        class="confirm-opt"
      >
        <input type="checkbox" :data-opt="o.key" :checked="o.checked" />
        <span>{{ o.label }}</span>
      </label>
      <div class="modal-actions">
        <button class="btn-secondary" @click="ui.resolveDialog(ui.dialog.options?.length ? { confirmed: false } : false)">{{ t('common.cancel') }}</button>
        <button class="btn-danger" @click="handleConfirm">{{ t('common.confirm') }}</button>
      </div>
    </div>

    <!-- Alert dialog -->
    <div v-else-if="ui.dialog.type === 'alert'" class="modal">
      <h3>{{ ui.dialog.title }}</h3>
      <p class="text-dim">{{ ui.dialog.message }}</p>
      <div class="modal-actions">
        <button class="btn-secondary" @click="ui.resolveDialog()">{{ t('common.ok') }}</button>
      </div>
    </div>

    <!-- Form modal -->
    <div v-else-if="ui.dialog.type === 'form'" class="modal">
      <h3>{{ ui.dialog.title }}</h3>
      <form @submit.prevent="handleFormSubmit">
        <div v-for="f in ui.dialog.fields" :key="f.key" class="field">
          <label>{{ f.label }}<span v-if="f.required" style="color:var(--danger)"> *</span></label>
          <select v-if="f.type === 'select' && f.options" :name="f.key" :required="f.required">
            <option value="">{{ t('common.pleaseSelect') }}</option>
            <option
              v-for="o in f.options"
              :key="o.value || o"
              :value="o.value || o"
              :selected="f.value === (o.value || o)"
            >{{ o.label || o }}</option>
          </select>
          <textarea
            v-else-if="f.type === 'textarea'"
            :name="f.key"
            :rows="f.rows || 3"
            :placeholder="f.placeholder || ''"
          >{{ f.value || '' }}</textarea>
          <input
            v-else
            :type="f.type === 'password' ? 'password' : 'text'"
            :name="f.key"
            :value="f.value || ''"
            :placeholder="f.placeholder || ''"
            :required="f.required"
            ref="firstInput"
          />
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="ui.resolveDialog(null)">{{ t('common.cancel') }}</button>
          <button type="submit">{{ ui.dialog.submitLabel || t('common.ok') }}</button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.confirm-opt {
  display: flex; align-items: center; gap: 8px;
  margin-top: 12px; font-size: 13px; cursor: pointer;
}
.confirm-opt input { width: auto; margin: 0; }
</style>
