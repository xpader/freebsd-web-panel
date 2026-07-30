<script setup>
import { ref, watch, computed } from 'vue';
import { useI18n } from 'vue-i18n';

const props = defineProps({
  // modelValue is an object { enabled: bool, expr: string }.
  modelValue: { type: Object, default: () => ({ enabled: false, expr: '' }) },
});
const emit = defineEmits(['update:modelValue']);
const { t } = useI18n();

const SPECIALS = ['@yearly', '@annually', '@monthly', '@weekly', '@daily', '@midnight', '@hourly'];

const enabled = ref(false);
const special = ref(''); // '' = custom 5-field
const minute = ref('*');
const hour = ref('*');
const dom = ref('*');
const month = ref('*');
const dow = ref('*');

const showCustomFields = computed(() => special.value === '');

// Parse the cron_expr string into local schedule fields (never touches enabled).
function parseExpr(expr) {
  const tok = (expr || '').trim().split(/\s+/).filter(Boolean);
  if (!tok.length) {
    special.value = '';
    minute.value = hour.value = dom.value = month.value = dow.value = '*';
    return;
  }
  if (SPECIALS.includes(tok[0])) {
    special.value = tok[0];
    minute.value = hour.value = dom.value = month.value = dow.value = '*';
  } else if (tok.length === 5) {
    special.value = '';
    [minute.value, hour.value, dom.value, month.value, dow.value] = tok;
  } else {
    special.value = '';
    minute.value = tok[0] || '*';
    hour.value = tok[1] || '*';
    dom.value = tok[2] || '*';
    month.value = tok[3] || '*';
    dow.value = tok[4] || '*';
  }
}

// Serialize local schedule fields back to a cron_expr string.
function serializeExpr() {
  if (special.value) return special.value;
  return [minute.value, hour.value, dom.value, month.value, dow.value]
    .map((f) => f.trim())
    .filter(Boolean)
    .join(' ');
}

// Emit the combined { enabled, expr } object.
function emitState() {
  emit('update:modelValue', { enabled: enabled.value, expr: serializeExpr() });
}

// Sync local state from incoming modelValue.
watch(() => props.modelValue, (v) => {
  const val = v || { enabled: false, expr: '' };
  enabled.value = !!val.enabled;
  parseExpr(val.expr || '');
}, { immediate: true, deep: true });

// Re-emit whenever any local field changes.
watch([enabled, special, minute, hour, dom, month, dow], emitState);
</script>

<template>
  <div class="cron-schedule">
    <label class="confirm-opt">
      <input type="checkbox" v-model="enabled" />
      <span>{{ t('rsync.enableSchedule') }}</span>
    </label>
    <div v-if="enabled" class="cron-schedule-body">
      <div class="field">
        <label>{{ t('cron.scheduleType') }}</label>
        <select v-model="special">
          <option value="">{{ t('cron.custom') }}</option>
          <option v-for="s in SPECIALS" :key="s" :value="s">{{ s }} — {{ t('cron.alias_' + s.replace('@', '')) }}</option>
        </select>
      </div>
      <div v-show="showCustomFields" class="cron-fields">
        <div class="field"><label>{{ t('cron.minute') }}</label><input v-model="minute" /></div>
        <div class="field"><label>{{ t('cron.hour') }}</label><input v-model="hour" /></div>
        <div class="field"><label>{{ t('cron.dom') }}</label><input v-model="dom" /></div>
        <div class="field"><label>{{ t('cron.month') }}</label><input v-model="month" /></div>
        <div class="field"><label>{{ t('cron.dow') }}</label><input v-model="dow" /></div>
        <p class="cron-help text-dim">{{ t('cron.fieldsHelp') }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cron-schedule {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.cron-schedule-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-left: 4px;
  border-left: 2px solid var(--border);
  padding-left: 12px;
}
.cron-fields {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 8px;
}
.cron-help {
  grid-column: 1 / -1;
  font-size: 12px;
  margin: 0;
}
@media (max-width: 600px) {
  .cron-fields {
    grid-template-columns: repeat(2, 1fr);
  }
}
</style>
