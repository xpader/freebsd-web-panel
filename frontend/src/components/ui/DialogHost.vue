<script setup>
import { ref, reactive, nextTick, watch, onUnmounted } from 'vue';
import { ui } from '../../stores/ui.js';
import { useI18n } from 'vue-i18n';
import FieldHelp from './FieldHelp.vue';
const { t } = useI18n();

const firstInput = ref(null);
const submitting = ref(false);
const radioState = reactive({});
const formValues = reactive({});

// Generic form-state reset when a form dialog opens.
watch(() => ui.dialog, (d) => {
  if (!d || d.type !== 'form') return;
  Object.keys(radioState).forEach(k => delete radioState[k]);
  Object.keys(formValues).forEach(k => delete formValues[k]);
  for (const f of (d.fields || [])) {
    formValues[f.key] = f.value ?? '';
    if (f.type === 'radio') radioState[f.key] = f.value ?? f.options?.[0]?.value ?? '';
  }
  nextTick(() => {
    if (firstInput.value) firstInput.value.focus();
  });
}, { immediate: true });

function handleConfirm(d) {
  if (d?.options?.length) {
    const result = { confirmed: true };
    for (const o of d.options) {
      const cb = document.querySelector(`[data-opt="${o.key}"]`);
      result[o.key] = cb ? cb.checked : false;
    }
    ui.resolveDialog(result);
  } else {
    ui.resolveDialog(true);
  }
}

async function handleFormSubmit(ev, d) {
  const result = { ...formValues, ...radioState };
  if (d.submitHandler) {
    submitting.value = true;
    d.errorMessage = null;
    try {
      await d.submitHandler(result);
      ui.resolveDialog(result);
    } catch (e) {
      d.errorMessage = e.message || String(e);
    } finally {
      submitting.value = false;
    }
  } else {
    ui.resolveDialog(result);
  }
}

function getFieldValue(key) {
  if (key in radioState) return radioState[key];
  return formValues[key];
}

function isFieldVisible(f) {
  if (!f.showIf) return true;
  const [key, val] = Object.entries(f.showIf)[0];
  const cur = getFieldValue(key);
  if (Array.isArray(val)) return val.includes(cur);
  return cur === val;
}

function isFieldRequired(f) {
  if (f.required) return true;
  if (f.requiredIf) {
    const [key, val] = Object.entries(f.requiredIf)[0];
    const cur = getFieldValue(key);
    if (Array.isArray(val)) return val.includes(cur);
    return cur === val;
  }
  return false;
}

function groupedFields(d) {
  const groups = [];
  let pending = null;
  for (const f of (d.fields || [])) {
    if (f.half) {
      if (pending) {
        groups.push([pending, f]);
        pending = null;
      } else {
        pending = f;
      }
    } else {
      if (pending) {
        groups.push([pending]);
        pending = null;
      }
      groups.push([f]);
    }
  }
  if (pending) groups.push([pending]);
  return groups;
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
        <button class="btn-danger" @click="handleConfirm(ui.dialog)">{{ t('common.confirm') }}</button>
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

    <!-- Code/preview dialog -->
    <div v-else-if="ui.dialog.type === 'code'" class="modal modal-wide">
      <h3>{{ ui.dialog.title }}</h3>
      <pre class="code-dialog-content mono">{{ ui.dialog.content }}</pre>
      <div class="modal-actions">
        <button class="btn-secondary" @click="ui.resolveDialog()">{{ t('common.close') }}</button>
      </div>
    </div>

    <!-- Countdown confirm dialog (pure render — state comes from dialog config) -->
    <div v-else-if="ui.dialog.type === 'countdown'" class="modal">
      <h3>{{ ui.dialog.title }}</h3>
      <p class="text-dim">{{ ui.dialog.message }}</p>
      <div class="countdown-bar-container">
        <div class="countdown-bar" :style="{ width: ui.dialog.pct + '%' }"></div>
      </div>
      <p style="text-align:center;font-size:28px;font-weight:bold;margin:12px 0;">
        {{ ui.dialog.secs }}s
      </p>
      <div v-if="ui.dialog.warning" class="countdown-warning">
        <i class="fa-solid fa-triangle-exclamation"></i>
        {{ ui.dialog.warning }}
      </div>
      <div class="modal-actions">
        <button class="btn-danger" @click="ui.resolveDialog('rollback')">
          <i class="fa-solid fa-rotate-left"></i>
          {{ ui.dialog.rollbackLabel || t('common.cancel') }}
        </button>
        <button @click="ui.resolveDialog('confirm')">
          <i class="fa-solid fa-check"></i>
          {{ ui.dialog.confirmLabel || t('common.confirm') }}
        </button>
      </div>
    </div>

    <!-- Form modal -->
    <div v-else-if="ui.dialog.type === 'form'" class="modal modal-wide">
      <h3>{{ ui.dialog.title }}</h3>
      <form @submit.prevent="handleFormSubmit($event, ui.dialog)">
        <!-- Group consecutive half-width fields into rows -->
        <template v-for="(group, gi) in groupedFields(ui.dialog)" :key="gi">
          <div :class="group.length > 1 ? 'form-row-half' : ''">
            <div v-for="f in group" :key="f.key" class="field" v-show="isFieldVisible(f)">
              <label :for="'field-' + f.key">
                {{ f.label }}
                <span v-if="f.help" class="field-help-inline">
                  <FieldHelp :text="f.help" />
                </span>
                <span v-if="isFieldRequired(f)" class="required-mark">*</span>
              </label>
              <!-- radio -->
              <div v-if="f.type === 'radio'" class="radio-group">
                <label v-for="opt in f.options" :key="opt.value" class="radio-label">
                  <input type="radio" :name="f.key" :value="opt.value" v-model="radioState[f.key]" />
                  <span>{{ opt.label }}</span>
                </label>
              </div>
              <!-- select -->
              <select v-else-if="f.type === 'select'" :id="'field-' + f.key" v-model="formValues[f.key]" :required="isFieldRequired(f)">
                <option value="" v-if="!f.required">{{ t('common.pleaseSelect') }}</option>
                <option v-for="opt in f.options" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
              </select>
              <!-- checkbox -->
              <div v-else-if="f.type === 'checkbox'" class="checkbox-field">
                <label>
                  <input type="checkbox" v-model="formValues[f.key]" />
                  <span>{{ f.checkboxLabel || '' }}</span>
                </label>
              </div>
              <!-- textarea -->
              <textarea v-else-if="f.type === 'textarea'" :id="'field-' + f.key" v-model="formValues[f.key]"
                :placeholder="f.placeholder || ''" :required="isFieldRequired(f)" rows="3"></textarea>
              <!-- text input -->
              <input v-else :id="'field-' + f.key" v-model.trim="formValues[f.key]"
                :type="f.inputType || 'text'"
                :placeholder="f.placeholder || ''"
                :required="isFieldRequired(f)" />
              <small v-if="f.hint" class="field-hint">{{ f.hint }}</small>
            </div>
          </div>
        </template>
        <div v-if="ui.dialog.errorMessage" class="form-error">{{ ui.dialog.errorMessage }}</div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="ui.resolveDialog(null)">{{ t('common.cancel') }}</button>
          <button type="submit" class="btn-primary" :disabled="submitting">{{ ui.dialog.submitLabel || t('common.confirm') }}</button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.countdown-bar-container {
  width: 100%;
  height: 8px;
  background: var(--bg-elev2);
  border-radius: 4px;
  overflow: hidden;
  margin: 16px 0 4px 0;
}
.countdown-bar {
  height: 100%;
  background: var(--accent);
  transition: width 0.5s linear;
  border-radius: 4px;
}
.countdown-warning {
  background: rgba(255, 180, 0, 0.12);
  border: 1px solid rgba(255, 180, 0, 0.4);
  color: #ffb400;
  padding: 10px 14px;
  border-radius: 6px;
  margin: 8px 0;
  font-size: 13px;
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
