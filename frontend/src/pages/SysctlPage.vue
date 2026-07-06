<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const PAGE_SIZE = 100;
const allEntries = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const page = ref(0);
const modFilter = ref('modified');
const wrFilter = ref('writable');
const search = ref('');

const filtered = computed(() => {
  const q = search.value.toLowerCase();
  return allEntries.value.filter((e) => {
    if (modFilter.value === 'modified' && !e.modified) return false;
    if (wrFilter.value === 'writable' && !e.writable) return false;
    if (wrFilter.value === 'readonly' && e.writable) return false;
    if (!q) return true;
    return e.name.toLowerCase().includes(q) || (e.value || '').toLowerCase().includes(q) || (e.description || '').toLowerCase().includes(q);
  });
});

const totalPages = computed(() => Math.ceil(filtered.value.length / PAGE_SIZE));
const pageItems = computed(() => {
  const start = page.value * PAGE_SIZE;
  return filtered.value.slice(start, start + PAGE_SIZE);
});

const rangeText = computed(() => {
  if (!filtered.value.length) return '0';
  const from = page.value * PAGE_SIZE + 1;
  const to = Math.min(filtered.value.length, page.value * PAGE_SIZE + PAGE_SIZE);
  return `${from}–${to}`;
});

async function load() {
  if (!allEntries.value.length) loading.value = true;
  refreshing.value = true;
  try {
    allEntries.value = await api.get('/api/sysctl');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function doEdit(entry) {
  const result = await formModal(t('sysctl.editTitle', { key: entry.name }), [
    { key: 'value', label: t('common.value'), value: entry.value || '', placeholder: '' },
  ], t('common.save'));
  if (!result) return;
  try {
    await api.put(`/api/sysctl/${encodeURIComponent(entry.name)}`, { value: result.value });
    toast.toast(t('sysctl.saved'));
    await load();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  }
}

async function doReset(entry) {
  if (!await confirm(t('sysctl.resetTitle'), t('sysctl.resetConfirm', { key: entry.name }))) return;
  try {
    await api.del(`/api/sysctl/${encodeURIComponent(entry.name)}`);
    toast.toast(t('sysctl.resetDone'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

function goPage(p) {
  if (p >= 0 && p < totalPages.value) page.value = p;
}

function onSearch() { page.value = 0; }
function setMod(v) { modFilter.value = v; page.value = 0; }
function setWr(v) { wrFilter.value = v; page.value = 0; }

function truncate(s) {
  if (!s) return '';
  return s.length > 80 ? s.slice(0, 80) + '…' : s;
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('sysctl.title') }}</h1>
    <p>{{ t('sysctl.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <input type="text" v-model="search" class="filter-input" :placeholder="t('sysctl.filter')" @input="onSearch" />
    <div class="filter-group">
      <button :class="['filter-btn', { active: modFilter === 'modified' }]" @click="setMod('modified')">{{ t('sysctl.modified') }}</button>
      <button :class="['filter-btn', { active: modFilter === 'all' }]" @click="setMod('all')">{{ t('common.all') }}</button>
    </div>
    <div class="filter-group">
      <button :class="['filter-btn', { active: wrFilter === 'writable' }]" @click="setWr('writable')">{{ t('sysctl.writable') }}</button>
      <button :class="['filter-btn', { active: wrFilter === 'readonly' }]" @click="setWr('readonly')">{{ t('sysctl.readonly') }}</button>
      <button :class="['filter-btn', { active: wrFilter === 'all' }]" @click="setWr('all')">{{ t('common.all') }}</button>
    </div>
    <span class="text-dim">{{ t('sysctl.count', { total: allEntries.length, range: rangeText, filtered: filtered.length }) }}</span>
    <div class="flex">
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th><th>{{ t('common.value') }}</th><th>{{ t('common.type') }}</th>
        <th>{{ t('common.description') }}</th><th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="5" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="5" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!pageItems.length"><td colspan="5" class="empty">{{ t('sysctl.noResults') }}</td></tr>
        <template v-else>
          <tr v-for="(e, i) in pageItems" :key="i" :class="{ 'row-modified': e.modified }">
            <td class="mono">
              <strong>{{ e.name }}</strong>
              <span v-if="e.modified" class="badge-modified" :title="t('sysctl.modifiedHint')">{{ t('sysctl.modified') }}</span>
              <span v-if="e.writable" class="badge-writable" :title="t('sysctl.writableHint')">{{ t('sysctl.writable') }}</span>
            </td>
            <td class="mono">
              <div class="cell-ellipsis" :title="e.value || ''">{{ truncate(e.value || '') || '—' }}</div>
            </td>
            <td>
              <span v-if="e.type" class="badge-type">{{ e.type }}</span>
              <span v-else class="text-dim">—</span>
            </td>
            <td><div class="cell-wrap text-dim">{{ e.description || '—' }}</div></td>
            <td>
              <div v-if="e.writable && e.value !== null || e.modified" class="btn-group">
                <button v-if="e.writable && e.value !== null" class="btn-secondary btn-sm" @click="doEdit(e)">{{ t('common.edit') }}</button>
                <button v-if="e.modified" class="btn-danger btn-sm" @click="doReset(e)">{{ t('sysctl.reset') }}</button>
              </div>
              <span v-else class="text-dim">—</span>
            </td>
          </tr>
        </template>
      </tbody>
    </table>
  </div>
  <div v-if="totalPages > 1" class="pagination">
    <button class="btn-secondary btn-sm" :disabled="page === 0" @click="goPage(page - 1)">{{ t('sysctl.prev') }}</button>
    <button v-for="p in totalPages" :key="p" :class="['btn-secondary', 'btn-sm', { active: p - 1 === page }]" @click="goPage(p - 1)">{{ p }}</button>
    <button class="btn-secondary btn-sm" :disabled="page >= totalPages - 1" @click="goPage(page + 1)">{{ t('sysctl.next') }}</button>
  </div>
</template>
