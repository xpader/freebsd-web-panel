<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useConfirm, useAlert } from '../composables/useDialog.js';
import SearchInput from '../components/ui/SearchInput.vue';

const { t } = useI18n();
const toast = useToast();
const confirm = useConfirm();
const alert = useAlert();

const SPECIALS = ['', '@reboot', '@yearly', '@annually', '@monthly', '@weekly', '@daily', '@midnight', '@hourly'];
const all = ref([]);
const targets = ref(null);
const filter = ref('');
const loading = ref(true);
const error = ref('');

// Modal state
const showModal = ref(false);
const isEdit = ref(false);
const editSource = ref('');
const editLine = ref(null);
const form = ref({});

function scheduleText(e) {
  if (e.kind === 'special') return e.special || '';
  return [e.minute, e.hour, e.dom, e.month, e.dow].filter((x) => x != null).join(' ');
}

function describe(e) {
  if (e.kind !== 'special' || !e.special) return '';
  return t('cron.alias_' + e.special.replace('@', ''));
}

function sourceTitle(src) {
  return src === 'system' ? '/etc/crontab' : src;
}

function orderedSources(entries) {
  const set = new Set(['system']);
  entries.forEach((e) => { if (e.source !== 'system') set.add(e.source); });
  return [...set].sort((a, b) => {
    if (a === 'system') return -1;
    if (b === 'system') return 1;
    return a.localeCompare(b);
  });
}

const filtered = computed(() => {
  const q = filter.value.toLowerCase();
  if (!q) return all.value;
  return all.value.filter((e) =>
    scheduleText(e).toLowerCase().includes(q) ||
    (e.command || '').toLowerCase().includes(q) ||
    (e.user || '').toLowerCase().includes(q) ||
    (e.comment || '').toLowerCase().includes(q) ||
    (e.source || '').toLowerCase().includes(q)
  );
});

async function load() {
  if (!all.value.length) loading.value = true;
  try {
    all.value = await api.get('/api/crontab');
    error.value = '';
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
}

async function loadTargets() {
  if (targets.value) return;
  try {
    targets.value = await api.get('/api/crontab/targets');
  } catch {
    targets.value = [{ source: 'system', label: '/etc/crontab' }];
  }
}

function openAdd(preselect) {
  isEdit.value = false;
  editSource.value = preselect || null;
  editLine.value = null;
  form.value = {
    target: preselect || 'system',
    special: '',
    minute: '*', hour: '*', dom: '*', month: '*', dow: '*',
    user: 'root', command: '', comment: '', disabled: false,
  };
  showModal.value = true;
}

function openEdit(source, line) {
  const entry = all.value.find((e) => e.source === source && e.line === line);
  if (!entry) return;
  isEdit.value = true;
  editSource.value = source;
  editLine.value = line;
  const sp = entry.kind === 'special' && entry.special ? entry.special : '';
  form.value = {
    target: source,
    special: sp,
    minute: entry.minute || '*',
    hour: entry.hour || '*',
    dom: entry.dom || '*',
    month: entry.month || '*',
    dow: entry.dow || '*',
    user: entry.user || 'root',
    command: entry.command || '',
    comment: entry.comment || '',
    disabled: entry.disabled || false,
  };
  showModal.value = true;
}

const showCustomFields = computed(() => form.value.special === '');
const showUserField = computed(() => form.value.target === 'system');

async function submit() {
  const source = isEdit.value ? editSource.value : form.value.target;
  const isSystem = source === 'system';
  const custom = form.value.special === '';
  const payload = {
    source,
    special: custom ? null : form.value.special,
    minute: custom ? form.value.minute.trim() : null,
    hour: custom ? form.value.hour.trim() : null,
    dom: custom ? form.value.dom.trim() : null,
    month: custom ? form.value.month.trim() : null,
    dow: custom ? form.value.dow.trim() : null,
    user: isSystem ? form.value.user.trim() : null,
    command: form.value.command,
    comment: form.value.comment,
    disabled: form.value.disabled,
  };
  showModal.value = false;
  try {
    if (isEdit.value) {
      await api.put('/api/crontab', { line: editLine.value, ...payload });
      toast.toast(t('cron.saved'));
    } else {
      await api.post('/api/crontab', payload);
      toast.toast(t('cron.added'));
    }
    await load();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  }
}

async function toggleEntry(source, line) {
  const entry = all.value.find((e) => e.source === source && e.line === line);
  if (!entry) return;
  try {
    await api.put('/api/crontab', {
      source, line,
      special: entry.special || null,
      minute: entry.minute || null, hour: entry.hour || null,
      dom: entry.dom || null, month: entry.month || null, dow: entry.dow || null,
      user: entry.user || null, command: entry.command, comment: entry.comment,
      disabled: !entry.disabled,
    });
    toast.toast(entry.disabled ? t('common.enabled') : t('common.disabled'));
    await load();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  }
}

async function deleteEntry(source, line) {
  const entry = all.value.find((e) => e.source === source && e.line === line);
  if (!entry) return;
  if (!await confirm(t('cron.deleteTitle'), t('cron.deleteConfirm', { sched: scheduleText(entry) }))) return;
  try {
    await api.del(`/api/crontab?source=${encodeURIComponent(source)}&line=${line}`);
    toast.toast(t('cron.deleted'));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

onMounted(async () => {
  await load();
  await loadTargets();
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('cron.title') }}</h1>
    <p>{{ t('cron.subtitle') }}</p>
    <p class="text-dim cron-note">{{ t('cron.backupNote') }}</p>
  </div>
  <div class="toolbar">
    <SearchInput v-model="filter" :placeholder="t('cron.filter')" />
    <span class="text-dim">{{ t('cron.count', { n: filtered.length }) }}</span>
    <div class="flex">
      <button @click="openAdd(null)"><i class="fa-solid fa-plus"></i> {{ t('cron.add') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('cron.schedule') }}</th><th>{{ t('common.user') }}</th><th>{{ t('cron.command') }}</th>
        <th>{{ t('cron.comment') }}</th><th>{{ t('common.status') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="6" class="empty">{{ t('cron.noEntries') }}</td></tr>
        <template v-else>
          <template v-for="src in orderedSources(filtered)" :key="src">
            <tr class="cron-section-row">
              <td colspan="6">
                <div class="cron-section">
                  <span class="cron-section-title">{{ sourceTitle(src) }}</span>
                  <span class="cron-section-sub text-dim">{{ t('cron.entriesCount', { n: filtered.filter(e => e.source === src).length }) }}</span>
                  <button class="btn-secondary btn-sm" @click="openAdd(src)">{{ t('cron.addIn') }}</button>
                </div>
              </td>
            </tr>
            <tr v-for="e in filtered.filter(en => en.source === src)" :key="e.source + '-' + e.line" :class="{ 'row-dim': e.disabled }">
              <td class="mono">
                <strong>{{ scheduleText(e) }}</strong>
                <span v-if="describe(e)" class="text-dim"><br>{{ describe(e) }}</span>
              </td>
              <td class="mono">{{ e.user || '—' }}</td>
              <td class="mono">
                <div class="cell-wrap">
                  <span v-if="e.system_task" class="badge badge-warn">{{ t('cron.systemTask') }} </span>
                  {{ e.command || '—' }}
                </div>
              </td>
              <td>
                <div v-if="e.comment" class="cell-wrap cron-comment">{{ e.comment }}</div>
                <span v-else class="text-dim">—</span>
              </td>
              <td>
                <span v-if="e.disabled" class="badge badge-dim">{{ t('common.disabled') }}</span>
                <span v-else class="badge badge-success">{{ t('common.enabled') }}</span>
              </td>
              <td>
                <div class="btn-group">
                  <button class="btn-secondary btn-sm" @click="openEdit(e.source, e.line)">{{ t('common.edit') }}</button>
                  <button class="btn-secondary btn-sm" @click="toggleEntry(e.source, e.line)">{{ e.disabled ? t('cron.enable') : t('cron.disable') }}</button>
                  <button class="btn-danger btn-sm" @click="deleteEntry(e.source, e.line)">{{ t('common.delete') }}</button>
                </div>
              </td>
            </tr>
          </template>
        </template>
      </tbody>
    </table>
  </div>

  <!-- Entry modal -->
  <div v-if="showModal" class="modal-overlay">
    <div class="modal">
      <h3>{{ isEdit ? t('cron.editTitle') : t('cron.addTitle') }}</h3>
      <form @submit.prevent="submit">
        <div class="field">
          <label>{{ t('cron.target') }}</label>
          <select v-model="form.target" :disabled="isEdit">
            <option v-for="tg in (targets || [])" :key="tg.source" :value="tg.source">{{ tg.label }}</option>
          </select>
        </div>
        <div class="field">
          <label>{{ t('cron.scheduleType') }}</label>
          <select v-model="form.special">
            <option value="">{{ t('cron.custom') }}</option>
            <option v-for="s in SPECIALS.filter(s => s)" :key="s" :value="s">{{ s }} — {{ t('cron.alias_' + s.replace('@', '')) }}</option>
          </select>
        </div>
        <div v-show="showCustomFields" class="cron-fields">
          <div class="field"><label>{{ t('cron.minute') }}</label><input v-model="form.minute" /></div>
          <div class="field"><label>{{ t('cron.hour') }}</label><input v-model="form.hour" /></div>
          <div class="field"><label>{{ t('cron.dom') }}</label><input v-model="form.dom" /></div>
          <div class="field"><label>{{ t('cron.month') }}</label><input v-model="form.month" /></div>
          <div class="field"><label>{{ t('cron.dow') }}</label><input v-model="form.dow" /></div>
          <p class="cron-help text-dim">{{ t('cron.fieldsHelp') }}</p>
        </div>
        <div v-show="showUserField" class="field">
          <label>{{ t('common.user') }} <span style="color:var(--danger)">*</span></label>
          <input v-model="form.user" placeholder="root" />
        </div>
        <div class="field">
          <label>{{ t('cron.command') }} <span style="color:var(--danger)">*</span></label>
          <input v-model="form.command" required placeholder="/usr/local/bin/backup.sh" />
        </div>
        <div class="field">
          <label>{{ t('cron.comment') }}</label>
          <textarea v-model="form.comment" rows="2" :placeholder="t('cron.commentPlaceholder')"></textarea>
        </div>
        <div class="field cron-check">
          <label><input type="checkbox" v-model="form.disabled" /> {{ t('cron.disabledHint') }}</label>
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showModal = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ isEdit ? t('common.save') : t('cron.add') }}</button>
        </div>
      </form>
    </div>
  </div>
</template>
