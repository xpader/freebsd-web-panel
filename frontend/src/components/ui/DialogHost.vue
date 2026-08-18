<script setup>
import { ref, reactive, nextTick, watch, onUnmounted } from 'vue';
import { ui } from '../../stores/ui.js';
import { useI18n } from 'vue-i18n';
import FieldHelp from './FieldHelp.vue';
import FilePicker from './FilePicker.vue';
import RemoteFilePicker from './RemoteFilePicker.vue';
import CronScheduleInput from './CronScheduleInput.vue';
import PermissionDialog from './PermissionDialog.vue';
const { t } = useI18n();
const submitting = ref(false);
const firstInput = ref(null);
const radioState = reactive({});
const formValues = reactive({});
const listQuery = reactive({});
const pickerField = ref(null);
const pickerRemote = ref(false);
const permField = ref(null);

/// True when a path spec looks like an SSH connection (`user@host` or `host:`).
function isRemoteSpec(v) {
  if (!v) return false;
  return /^[^/\s]+@/.test(v) || (!v.startsWith('/') && v.includes(':'));
}

/// Open the picker for a field, auto-detecting local vs remote from its value.
/// `f.portKey` (optional) names another field whose value holds the SSH port.
function openPicker(f) {
  pickerField.value = f.key;
  pickerRemote.value = isRemoteSpec(formValues[f.key]);
}

// Generic form-state reset when a form dialog opens.
watch(() => ui.dialog, (d) => {
  if (!d || d.type !== 'form') return;
  Object.keys(listQuery).forEach(k => delete listQuery[k]);
  Object.keys(formValues).forEach(k => delete formValues[k]);
  for (const f of (d.fields || [])) {
    formValues[f.key] = f.value ?? '';
    if (f.type === 'radio') radioState[f.key] = f.value ?? f.options?.[0]?.value ?? '';
    if (f.type === 'list-select') listQuery[f.key] = '';
    if (f.type === 'checkbox-group') {
      for (const opt of (f.options || [])) {
        formValues[opt.key] = opt.value ?? false;
      }
    }
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


/// Filter list-select options by the current search query.
function filterListOptions(f) {
  const q = (listQuery[f.key] || '').toLowerCase();
  if (!q) return f.options || [];
  return (f.options || []).filter(o => o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q));
}

/// Get the display label of the currently selected option in a list-select.
function selectedListLabel(f) {
  const opt = (f.options || []).find(o => o.value === formValues[f.key]);
  return opt ? opt.label : formValues[f.key];
}

function groupedFields(d) {
  const groups = [];
  const rowMap = new Map();
  let pending = null;
  for (const f of (d.fields || [])) {
    if (f.half) {
      if (f.row != null) {
        if (pending) {
          groups.push([pending]);
          pending = null;
        }
        if (rowMap.has(f.row)) {
          rowMap.get(f.row).push(f);
        } else {
          const g = [f];
          groups.push(g);
          rowMap.set(f.row, g);
        }
      } else if (pending) {
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
      <p class="text-dim" style="white-space:pre-line;">{{ ui.dialog.message }}</p>
      <div class="modal-actions">
        <button class="btn-secondary" @click="ui.resolveDialog(false)">{{ ui.dialog.dismissLabel || t('common.ok') }}</button>
        <button v-if="ui.dialog.actionLabel" @click="ui.resolveDialog(true)">{{ ui.dialog.actionLabel }}</button>
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
              <label v-if="f.label" :for="f.type === 'checkbox' ? null : ('field-' + f.key)">
                {{ f.label }}
                <span v-if="f.help" class="field-help-inline">
                  <FieldHelp :text="f.help" />
                </span>
                <span v-if="isFieldRequired(f)" class="required-mark">*</span>
              </label>
              <!-- radio (pill style) -->
              <div v-if="f.type === 'radio'" class="radio-pill-group">
                <label
                  v-for="opt in f.options"
                  :key="opt.value"
                  class="radio-pill"
                  :class="{ active: radioState[f.key] === opt.value, disabled: f.disabled }"
                >
                  <input type="radio" :name="f.key" :value="opt.value" v-model="radioState[f.key]" :disabled="f.disabled" />
                  <span>{{ opt.label }}</span>
                </label>
              </div>
              <!-- checkbox-group (pill style, multiple) -->
              <div v-else-if="f.type === 'checkbox-group'" class="radio-pill-group">
                <label
                  v-for="opt in f.options"
                  :key="opt.key"
                  class="radio-pill"
                  :class="{ active: formValues[opt.key] }"
                >
                  <input type="checkbox" v-model="formValues[opt.key]" />
                  <span>{{ opt.label }}</span>
                  <FieldHelp v-if="opt.help" :text="opt.help" />
                </label>
              </div>
              <!-- select -->
              <select v-else-if="f.type === 'select'" :id="'field-' + f.key" v-model="formValues[f.key]" :required="isFieldRequired(f)" :disabled="f.disabled">
                <option value="" v-if="!f.required">{{ t('common.pleaseSelect') }}</option>
                <option v-for="opt in f.options" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
              </select>
              <!-- list-select (search + pickable list) -->
              <div v-else-if="f.type === 'list-select'">
                <input type="text" v-model="listQuery[f.key]" class="list-select-search"
                  :placeholder="t('common.search')" />
                <div v-if="formValues[f.key]" class="list-select-chosen">
                  <i class="fa-solid fa-check"></i> {{ selectedListLabel(f) }}
                </div>
                <div class="list-select-box">
                  <div v-for="opt in filterListOptions(f)" :key="opt.value"
                    class="list-select-item"
                    :class="{ active: formValues[f.key] === opt.value }"
                    @click="formValues[f.key] = opt.value">
                    <i v-if="formValues[f.key] === opt.value" class="fa-solid fa-check list-select-check"></i>
                    <i v-else class="list-select-check-placeholder"></i>
                    <span>{{ opt.label }}</span>
                    <span v-if="opt.meta" class="list-select-meta">{{ opt.meta }}</span>
                  </div>
                  <div v-if="!filterListOptions(f).length" class="list-select-empty">{{ t('common.noData') }}</div>
                </div>
              </div>
              <!-- checkbox -->
              <label v-else-if="f.type === 'checkbox'" class="confirm-opt">
                <input type="checkbox" v-model="formValues[f.key]" />
                <span>{{ f.desc || f.label }}</span>
              </label>
              <!-- textarea -->
              <textarea v-else-if="f.type === 'textarea'" :id="'field-' + f.key" v-model="formValues[f.key]"
                :placeholder="f.placeholder || ''" :required="isFieldRequired(f)" rows="3"></textarea>
              <!-- path picker -->
              <div v-else-if="f.picker" class="input-with-btn">
                <input :id="'field-' + f.key" v-model="formValues[f.key]"
                  :type="f.inputType || 'text'"
                  :placeholder="f.placeholder || ''"
                  :required="isFieldRequired(f)" :disabled="f.disabled" />
                <button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker(f)">
                  <i :class="isRemoteSpec(formValues[f.key]) ? 'fa-solid fa-globe' : 'fa-solid fa-folder-open'"></i>
                </button>
              </div>
              <!-- cron schedule (single cron_expr string) -->
              <CronScheduleInput v-else-if="f.type === 'cron'" v-model="formValues[f.key]" />
              <!-- octal permission picker (input + button → opens PermissionDialog) -->
              <div v-else-if="f.permPicker" class="input-with-btn">
                <input :id="'field-' + f.key" v-model.trim="formValues[f.key]"
                  :placeholder="f.placeholder || '0644'"
                  :required="isFieldRequired(f)" :disabled="f.disabled" />
                <button type="button" class="btn-secondary btn-sm" :title="t('fm.editPermissions')" @click="permField = f.key">
                  <i class="fa-solid fa-sliders"></i>
                </button>
              </div>
              <!-- text input -->
              <input v-else :id="'field-' + f.key" v-model.trim="formValues[f.key]"
                :type="f.inputType || 'text'"
                :placeholder="f.placeholder || ''"
                :required="isFieldRequired(f)" :disabled="f.disabled" />
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
  <!-- Local directory picker -->
  <FilePicker
    v-if="pickerField && !pickerRemote"
    :mode="(ui.dialog.fields.find(f => f.key === pickerField)?.picker) || 'dir'"
    :initial-path="formValues[pickerField] || '/'"
    @select="(p) => { formValues[pickerField] = p; pickerField = null; }"
    @close="pickerField = null"
  />
  <!-- Remote directory picker (SSH) -->
  <RemoteFilePicker
    v-if="pickerField && pickerRemote"
    :initial-spec="formValues[pickerField] || ''"
    :port="formValues[(ui.dialog.fields.find(f => f.key === pickerField)?.portKey)] || 22"
    @select="(p) => { formValues[pickerField] = p; pickerField = null; }"
    @close="pickerField = null"
  />
  <!-- Octal permission picker -->
  <PermissionDialog
    v-if="permField"
    :title="ui.dialog.fields.find(f => f.key === permField)?.label || t('fm.editPermissions')"
    :value="formValues[permField] || '0644'"
    :special="ui.dialog.fields.find(f => f.key === permField)?.special !== false"
    @confirm="(v) => { formValues[permField] = v; permField = null; }"
    @close="permField = null"
  />
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
.list-select-search {
  width: 100%;
  margin-bottom: .5rem;
}
.list-select-chosen {
  display: flex;
  align-items: center;
  gap: .4rem;
  padding: .4rem .75rem;
  margin-bottom: .5rem;
  background: #e0f0ec;
  border-radius: 4px;
  font-size: .85rem;
  font-weight: 600;
  color: #2a7a6a;
}
.list-select-box {
  height: 280px;
  overflow-y: auto;
  border: 1px solid var(--border-dim, rgba(0,0,0,.12));
  border-radius: 6px;
  background: #fff;
}
.list-select-item {
  display: flex;
  align-items: center;
  gap: .5rem;
  padding: .4rem .75rem;
  cursor: pointer;
  font-size: .85rem;
  color: #333;
}
.list-select-item:hover {
  background: #f0f0f0;
}
.list-select-item.active {
  background: #e0f0ec;
  color: #2a7a6a;
  font-weight: 600;
}
.list-select-meta {
  margin-left: auto;
  color: #999;
  font-size: .8rem;
  font-weight: 400;
}
.list-select-check {
  color: #2a7a6a;
  width: 14px;
  flex-shrink: 0;
}
.list-select-check-placeholder {
  width: 14px;
  flex-shrink: 0;
}
.list-select-empty {
  padding: 1rem;
  text-align: center;
  color: #999;
}
</style>
