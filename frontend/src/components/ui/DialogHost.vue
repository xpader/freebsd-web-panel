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

// Countdown dialog state
const countdownSecs = ref(0);
const countdownPct = ref(100);
let countdownTimer = null;

watch(() => ui.dialog, (d) => {
  if (countdownTimer) { clearInterval(countdownTimer); countdownTimer = null; }
  if (!d) return;
  if (d.type === 'countdown') {
    const total = d.timeoutSeconds || 60;
    const endTime = d.expiresAt * 1000;
    countdownSecs.value = Math.max(0, Math.ceil((endTime - Date.now()) / 1000));
    countdownPct.value = (countdownSecs.value / total) * 100;
    countdownTimer = setInterval(() => {
      const remaining = Math.max(0, Math.ceil((endTime - Date.now()) / 1000));
      countdownSecs.value = remaining;
      countdownPct.value = (remaining / total) * 100;
      if (remaining <= 0) {
        clearInterval(countdownTimer);
        countdownTimer = null;
        ui.resolveDialog('rollback');
      }
    }, 500);
  }
}, { immediate: true });

onUnmounted(() => {
  if (countdownTimer) clearInterval(countdownTimer);
});

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

    <!-- Countdown confirm dialog -->
    <div v-else-if="ui.dialog.type === 'countdown'" class="modal">
      <h3>{{ ui.dialog.title }}</h3>
      <p class="text-dim">{{ ui.dialog.message }}</p>
      <div class="countdown-bar-container">
        <div class="countdown-bar" :style="{ width: countdownPct + '%' }"></div>
      </div>
      <p style="text-align:center;font-size:28px;font-weight:bold;margin:12px 0;">
        {{ countdownSecs }}s
      </p>
      <div class="modal-actions">
        <button class="btn-danger" @click="ui.resolveDialog('rollback')">
          <i class="fa-solid fa-rotate-left"></i> {{ t('firewall.rollbackNow') }}
        </button>
        <button @click="ui.resolveDialog('confirm')">
          <i class="fa-solid fa-check"></i> {{ t('firewall.keepChanges') }}
        </button>
      </div>
    </div>

    <!-- Form modal -->
    <div v-else-if="ui.dialog.type === 'form'" class="modal">
      <h3>{{ ui.dialog.title }}</h3>
      <form @submit.prevent="handleFormSubmit($event, ui.dialog)">
        <!-- Group consecutive half-width fields into rows -->
        <template v-for="(group, gi) in groupedFields(ui.dialog)" :key="gi">
          <div v-if="group.length > 1" class="form-row-half">
            <div v-for="f in group" :key="f.key" class="field" v-show="isFieldVisible(f)">
              <label>{{ f.label }}<span v-if="isFieldRequired(f)" style="color:var(--danger)"> *</span>
                <FieldHelp v-if="f.tooltip" :text="f.tooltip" />
              </label>
              <select v-if="f.type === 'select' && f.options" v-model="formValues[f.key]" :name="f.key" :required="isFieldRequired(f)" :disabled="!isFieldVisible(f)">
                <option v-if="!f.options.some(o => (o.value ?? o) === '')" value="">{{ t('common.pleaseSelect') }}</option>
                <option
                  v-for="o in f.options"
                  :key="o.value ?? o"
                  :value="o.value ?? o"
                >{{ o.label || o }}</option>
              </select>
              <div v-else-if="f.type === 'radio' && f.options" class="radio-group">
                <label
                  v-for="o in f.options"
                  :key="o.value ?? o"
                  class="radio-item"
                  :class="{ active: radioState[f.key] === (o.value ?? o) }"
                >
                  <input type="radio" :name="f.key" :value="o.value ?? o" :checked="radioState[f.key] === (o.value ?? o)" @change="radioState[f.key] = o.value ?? o" />
                  <span>{{ o.label || o }}</span>
                </label>
              </div>
              <textarea
                v-else-if="f.type === 'textarea'"
                v-model="formValues[f.key]"
                :name="f.key"
                :rows="f.rows || 3"
                :placeholder="f.placeholder || ''"
                :required="isFieldRequired(f)"
                :disabled="!isFieldVisible(f)"
              />
              <input
                v-else
                v-model="formValues[f.key]"
                :type="f.type === 'password' ? 'password' : 'text'"
                :name="f.key"
                :placeholder="f.placeholder || ''"
                :required="isFieldRequired(f)"
                :disabled="!isFieldVisible(f)"
                ref="firstInput"
              />
            </div>
          </div>
          <div v-else class="field" v-show="isFieldVisible(group[0])">
            <label>{{ group[0].label }}<span v-if="isFieldRequired(group[0])" style="color:var(--danger)"> *</span>
              <FieldHelp v-if="group[0].tooltip" :text="group[0].tooltip" />
            </label>
            <select v-if="group[0].type === 'select' && group[0].options" v-model="formValues[group[0].key]" :name="group[0].key" :required="isFieldRequired(group[0])" :disabled="!isFieldVisible(group[0])">
              <option v-if="!group[0].options.some(o => (o.value ?? o) === '')" value="">{{ t('common.pleaseSelect') }}</option>
              <option
                v-for="o in group[0].options"
                :key="o.value ?? o"
                :value="o.value ?? o"
              >{{ o.label || o }}</option>
            </select>
            <div v-else-if="group[0].type === 'radio' && group[0].options" class="radio-group">
              <label
                v-for="o in group[0].options"
                :key="o.value ?? o"
                class="radio-item"
                :class="{ active: radioState[group[0].key] === (o.value ?? o) }"
              >
                <input type="radio" :name="group[0].key" :value="o.value ?? o" :checked="radioState[group[0].key] === (o.value ?? o)" @change="radioState[group[0].key] = o.value ?? o" />
                <span>{{ o.label || o }}</span>
              </label>
            </div>
            <textarea
              v-else-if="group[0].type === 'textarea'"
              v-model="formValues[group[0].key]"
              :name="group[0].key"
              :rows="group[0].rows || 3"
              :placeholder="group[0].placeholder || ''"
              :required="isFieldRequired(group[0])"
              :disabled="!isFieldVisible(group[0])"
            />
            <input
              v-else
              v-model="formValues[group[0].key]"
              :type="group[0].type === 'password' ? 'password' : 'text'"
              :name="group[0].key"
              :placeholder="group[0].placeholder || ''"
              :required="isFieldRequired(group[0])"
              :disabled="!isFieldVisible(group[0])"
              ref="firstInput"
            />
          </div>
        </template>
        <div v-if="ui.dialog.errorMessage" class="form-error">{{ ui.dialog.errorMessage }}</div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="ui.resolveDialog(null)">{{ t('common.cancel') }}</button>
          <button type="submit" :disabled="submitting">
            <span v-if="submitting" class="spinner" style="width:14px;height:14px;"></span>
            {{ submitting ? '' : (ui.dialog.submitLabel || t('common.ok')) }}
          </button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.confirm-opt {
  display: flex !important; align-items: center; gap: 8px;
  margin-top: 0; margin-bottom: 0; font-size: 13px; cursor: pointer;
  padding: 6px 12px; border-radius: var(--radius);
  transition: background 0.15s;
}
.confirm-opt:first-of-type { margin-top: 12px; }
.confirm-opt:hover { background: var(--bg-elev2); }
.confirm-opt input { width: auto; margin: 0; }
.radio-group { display: flex; flex-direction: row; gap: 8px; }
.radio-item {
  display: inline-flex !important; align-items: center; gap: 6px;
  padding: 6px 14px; font-size: 13px; cursor: pointer;
  border-radius: var(--radius);
  margin-bottom: 0 !important;
  transition: background 0.15s, color 0.15s;
}
.radio-item:hover { background: var(--bg-elev2); }
.radio-item input { width: auto; margin: 0; }
.radio-item.active {
  color: var(--accent); background: rgba(59,130,246,0.08);
}
.form-error {
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid var(--danger);
  border-radius: var(--radius);
  padding: 8px 12px;
  margin-bottom: 16px;
  font-size: 13px;
  color: var(--danger);
}
.form-row-half {
  display: flex;
  gap: 12px;
}
.form-row-half .field {
  flex: 1;
}
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
</style>
