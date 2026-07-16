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

const tables = ref([]);
const loading = ref(true);
const expandedId = ref(null);
const newEntryAddr = ref({});

async function loadTables() {
  loading.value = true;
  try {
    tables.value = await api.get('/api/firewall/tables');
  } catch (e) {
    tables.value = [];
  } finally {
    loading.value = false;
  }
}

function toggleExpand(table) {
  expandedId.value = expandedId.value === table.id ? null : table.id;
}

async function doCreateTable() {
  const result = await formModal(t('firewall.addTableTitle'), [
    {
      key: 'name',
      label: t('common.name'),
      value: '',
      placeholder: 'blocked_ips',
      required: true,
    },
    {
      key: 'description',
      label: t('common.description'),
      value: '',
      placeholder: t('firewall.tableDescPlaceholder'),
    },
  ], { submitLabel: t('common.create') });

  if (!result) return;
  try {
    await api.post('/api/firewall/tables', { name: result.name, description: result.description || null });
    toast.toast(t('firewall.tableAdded'));
    await loadTables();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doEditTable(table) {
  const result = await formModal(t('firewall.editTableTitle'), [
    {
      key: 'name',
      label: t('common.name'),
      value: table.name,
      required: true,
    },
    {
      key: 'description',
      label: t('common.description'),
      value: table.description || '',
    },
  ], { submitLabel: t('common.save') });

  if (!result) return;
  try {
    await api.put(`/api/firewall/tables/${table.id}`, { name: result.name, description: result.description || null });
    toast.toast(t('common.saved'));
    await loadTables();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doDeleteTable(table) {
  if (!await confirm(t('firewall.deleteTableTitle'), t('firewall.deleteTableConfirm', { name: table.name }))) return;
  try {
    await api.del(`/api/firewall/tables/${table.id}`);
    toast.toast(t('firewall.tableDeleted'));
    if (expandedId.value === table.id) expandedId.value = null;
    await loadTables();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

async function doAddEntry(table) {
  const addr = (newEntryAddr.value[table.id] || '').trim();
  if (!addr) return;
  try {
    await api.post(`/api/firewall/tables/${table.id}/entries`, { address: addr });
    newEntryAddr.value[table.id] = '';
    await loadTables();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doDeleteEntry(table, entry) {
  try {
    await api.del(`/api/firewall/tables/${table.id}/entries/${entry.id}`);
    await loadTables();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

onMounted(() => {
  loadTables();
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <h1>{{ t('nav.firewallTables') }}</h1>
      <p class="text-dim" style="margin:0;font-size:13px;">{{ t('firewall.tablesSubtitle') }}</p>
    </div>
    <div class="flex btn-group" style="margin-left:auto;">
      <button @click="doCreateTable">
        <i class="fa-solid fa-plus"></i> {{ t('firewall.addTable') }}
      </button>
      <button class="btn-secondary" @click="loadTables">
        <i class="fa-solid fa-rotate"></i> {{ t('common.refresh') }}
      </button>
    </div>
  </div>

  <div v-if="loading" class="card">
    <div class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  </div>

  <div v-else-if="!tables.length" class="card">
    <div class="empty">{{ t('firewall.noTables') }}</div>
  </div>

  <template v-else>
    <div v-for="table in tables" :key="table.id" class="card fw-table-card">
      <div class="fw-table-header" @click="toggleExpand(table)">
        <div class="flex" style="align-items:center;gap:12px;">
          <i :class="['fa-solid', expandedId === table.id ? 'fa-chevron-down' : 'fa-chevron-right']"
             class="text-dim" style="font-size:12px;cursor:pointer;"></i>
          <span class="mono fw-table-name">&lt;{{ table.name }}&gt;</span>
          <span class="badge badge-muted">{{ table.entries.length }}</span>
          <span v-if="table.description" class="text-dim" style="font-size:13px;">{{ table.description }}</span>
        </div>
        <div class="btn-group" @click.stop>
          <button class="btn-secondary btn-sm" @click="doEditTable(table)">{{ t('common.edit') }}</button>
          <button class="btn-danger btn-sm" @click="doDeleteTable(table)">{{ t('common.delete') }}</button>
        </div>
      </div>

      <div v-if="expandedId === table.id" class="fw-table-entries">
        <div v-if="table.entries.length" class="fw-entry-list">
          <div v-for="entry in table.entries" :key="entry.id" class="fw-entry-row">
            <span class="mono">{{ entry.address }}</span>
            <button class="btn-danger btn-sm" @click="doDeleteEntry(table, entry)">
              <i class="fa-solid fa-xmark"></i>
            </button>
          </div>
        </div>
        <div v-else class="empty" style="padding:12px;">{{ t('firewall.noEntries') }}</div>
        <div class="fw-add-entry">
          <input
            type="text"
            class="input"
            :placeholder="t('firewall.entryPlaceholder')"
            v-model="newEntryAddr[table.id]"
            @keyup.enter="doAddEntry(table)"
            style="flex:1;"
          />
          <button @click="doAddEntry(table)"><i class="fa-solid fa-plus"></i> {{ t('common.add') }}</button>
        </div>
      </div>
    </div>
  </template>
</template>

<style scoped>
.fw-table-card {
  padding: 0;
}
.fw-table-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  cursor: pointer;
}
.fw-table-name {
  font-weight: 600;
  font-size: 15px;
}
.fw-table-entries {
  border-top: 1px solid var(--border);
  padding: 12px 16px;
}
.fw-entry-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}
.fw-entry-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px 4px 12px;
  border-radius: var(--radius);
  background: var(--bg-elev2);
  font-size: 13px;
}
.fw-add-entry {
  display: flex;
  gap: 8px;
  align-items: center;
}
</style>
