<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import SearchInput from '../components/ui/SearchInput.vue';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const allGroups = ref([]);
const loading = ref(true);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return allGroups.value;
  return allGroups.value.filter((g) =>
    g.name.toLowerCase().includes(q) ||
    String(g.gid).includes(q) ||
    g.members.some((m) => m.toLowerCase().includes(q))
  );
});

async function load() {
  loading.value = true;
  error.value = '';
  try {
    allGroups.value = await api.get('/api/accounts/groups');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
}

function parseMembers(str) {
  return (str || '').split(',').map((s) => s.trim()).filter(Boolean);
}

// System groups (gid < 1000) ship with the OS and are protected from
// deletion and identity-field changes.
function isSystemGroup(g) {
  return g.gid < 1000;
}

// ── create ──────────────────────────────────────────────────────────
async function addGroup() {
  const result = await formModal(
    t('accounts.addGroup'),
    [
      { key: 'name', label: t('common.name'), required: true, half: true },
      { key: 'gid', label: 'GID', inputType: 'number', placeholder: t('accounts.autoIdHint'), half: true },
      { key: 'members', label: t('accounts.members'), placeholder: t('accounts.memberHint'), help: t('accounts.memberHint') },
    ],
    { submitLabel: t('common.create') },
  );
  if (!result) return;

  const payload = { name: result.name.trim() };
  if (result.gid) payload.gid = Number(result.gid);
  const members = parseMembers(result.members);
  if (members.length) payload.members = members;

  try {
    await api.post('/api/accounts/groups', payload);
    toast.toast(t('accounts.groupCreated', { name: payload.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// ── edit ────────────────────────────────────────────────────────────
async function editGroup(g) {
  const result = await formModal(
    t('accounts.editGroup') + ' — ' + g.name,
    [
      { key: 'new_name', label: t('common.name'), value: g.name, required: true, half: true, disabled: isSystemGroup(g) },
      { key: 'gid', label: 'GID', inputType: 'number', value: String(g.gid), placeholder: t('accounts.autoIdHint'), half: true, disabled: isSystemGroup(g) },
      { key: 'members', label: t('accounts.members'), value: (g.members || []).join(', '), placeholder: t('accounts.memberHint'), help: t('accounts.memberHint') },
    ],
    { submitLabel: t('common.save') },
  );
  if (!result) return;

  const payload = {};
  if (!isSystemGroup(g)) {
    if (result.new_name && result.new_name !== g.name) payload.new_name = result.new_name.trim();
    if (result.gid) payload.gid = Number(result.gid);
  }
  payload.members = parseMembers(result.members);

  try {
    await api.put(`/api/accounts/groups/${encodeURIComponent(g.name)}`, payload);
    toast.toast(t('accounts.groupUpdated', { name: g.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// ── delete ──────────────────────────────────────────────────────────
async function deleteGroup(g) {
  if (!await confirm(t('accounts.deleteGroup'), t('accounts.deleteGroupConfirm', { name: g.name }))) return;
  try {
    await api.del(`/api/accounts/groups/${encodeURIComponent(g.name)}`);
    toast.toast(t('accounts.groupDeleted', { name: g.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('accounts.groupsTitle') }}</h1>
    <p>{{ t('accounts.groupsSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <SearchInput v-model="filter" :placeholder="t('accounts.filterGroup')" />
    <span class="text-dim">{{ t('accounts.groupCount', { n: filtered.length }) }}</span>
    <button style="margin-left:auto;" @click="addGroup">
      <i class="fa-solid fa-plus"></i> {{ t('accounts.addGroup') }}
    </button>
  </div>
  <div class="card table-wrap" style="padding:0;">
    <table>
      <thead>
        <tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('accounts.gid') }}</th>
          <th>{{ t('accounts.members') }}</th>
          <th>{{ t('common.actions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-if="error"><td colspan="4" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="4" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!filtered.length"><td colspan="4" class="empty">{{ t('accounts.noMatchGroup') }}</td></tr>
        <tr v-for="g in filtered" :key="g.gid">
          <td><strong>{{ g.name }}</strong></td>
          <td class="mono">{{ g.gid }}</td>
          <td>
            <template v-if="g.members.length">
              <span v-for="m in g.members" :key="m" class="badge badge-dim">{{ m }}</span>
            </template>
            <span v-else class="text-dim">—</span>
          </td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="editGroup(g)">{{ t('common.edit') }}</button>
              <button class="btn-danger btn-sm" :disabled="isSystemGroup(g)" @click="deleteGroup(g)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
