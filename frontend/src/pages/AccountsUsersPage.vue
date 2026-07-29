<script>
// Module-level cache for form options — shared across component instances,
// survives SPA navigation (component unmount/remount) within one page load.
let cachedGroups = null;
let cachedShells = null;
</script>

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

const allUsers = ref([]);
const allGroups = ref([]);
const shells = ref([]);
const loading = ref(true);
const error = ref('');
const filter = ref('');

const filtered = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return allUsers.value;
  return allUsers.value.filter((u) =>
    u.name.toLowerCase().includes(q) ||
    String(u.uid).includes(q) ||
    (u.group_name || '').toLowerCase().includes(q)
  );
});

async function load() {
  loading.value = true;
  error.value = '';
  try {
    allUsers.value = await api.get('/api/accounts/users');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
}

// Group/shell options are only used by the create/edit form dropdowns.
// Lazy-load on first form open and cache at module scope so they survive
// SPA navigation and aren't re-fetched on list refresh.
async function ensureOptions() {
  if (cachedGroups !== null && cachedShells !== null) {
    allGroups.value = cachedGroups;
    shells.value = cachedShells;
    return;
  }
  try {
    const [groups, sh] = await Promise.all([
      api.get('/api/accounts/groups'),
      api.get('/api/accounts/shells'),
    ]);
    cachedGroups = groups;
    cachedShells = sh;
    allGroups.value = groups;
    shells.value = sh;
  } catch {
    // Non-fatal: dropdowns will be empty; the form still works.
  }
}

const groupOptions = computed(() =>
  allGroups.value.map((g) => ({ value: g.name, label: `${g.name} (${g.gid})` }))
);
const shellOptions = computed(() => shells.value.map((s) => ({ value: s, label: s })));

function parseGroups(str) {
  return (str || '').split(',').map((s) => s.trim()).filter(Boolean);
}

// System accounts (uid < 1000) ship with the OS and are protected from
// deletion and identity-field changes.
function isSystemUser(u) {
  return u.uid < 1000;
}

// ── create ──────────────────────────────────────────────────────────
async function addUser() {
  await ensureOptions();
  const result = await formModal(
    t('accounts.addUser'),
    [
      { key: 'name', label: t('auth.username'), required: true, half: true },
      { key: 'uid', label: 'UID', inputType: 'number', placeholder: t('accounts.autoIdHint'), half: true },
      { key: 'gid', label: t('accounts.group'), type: 'select', options: groupOptions.value },
      { key: 'groups', label: t('accounts.supplementaryGroups'), placeholder: t('accounts.supplementaryGroupsHint'), help: t('accounts.supplementaryGroupsHint') },
      { key: 'gecos', label: t('common.description') },
      { key: 'home', label: t('accounts.home'), placeholder: '/home/…', picker: true },
      { key: 'create_home', label: '', desc: t('accounts.createHome'), type: 'checkbox' },
      { key: 'shell', label: 'Shell', type: 'select', options: shellOptions.value },
      { key: 'password', label: t('auth.password'), inputType: 'password', help: t('accounts.passwordHint') },
    ],
    { submitLabel: t('common.create') },
  );
  if (!result) return;

  const payload = { name: result.name.trim() };
  if (result.uid) payload.uid = Number(result.uid);
  if (result.gid) payload.gid = result.gid;
  if (result.gecos) payload.gecos = result.gecos;
  if (result.home) payload.home = result.home;
  if (result.shell) payload.shell = result.shell;
  const groups = parseGroups(result.groups);
  if (groups.length) payload.groups = groups;
  if (result.password) payload.password = result.password;
  payload.create_home = !!result.create_home;

  try {
    await api.post('/api/accounts/users', payload);
    toast.toast(t('accounts.userCreated', { name: payload.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// ── edit ────────────────────────────────────────────────────────────
async function editUser(u) {
  await ensureOptions();
  const result = await formModal(
    t('accounts.editUser') + ' — ' + u.name,
    [
      { key: 'new_name', label: t('auth.username'), value: u.name, required: true, half: true, disabled: isSystemUser(u) },
      { key: 'uid', label: 'UID', inputType: 'number', value: String(u.uid), placeholder: t('accounts.autoIdHint'), half: true, disabled: isSystemUser(u) },
      { key: 'gid', label: t('accounts.group'), type: 'select', value: u.group_name || '', options: groupOptions.value, disabled: isSystemUser(u) },
      { key: 'groups', label: t('accounts.supplementaryGroups'), value: (u.groups || []).join(', '), placeholder: t('accounts.supplementaryGroupsHint'), help: t('accounts.supplementaryGroupsHint') },
      { key: 'gecos', label: t('common.description'), value: u.gecos || '' },
      { key: 'home', label: t('accounts.home'), value: u.home || '', picker: true },
      { key: 'shell', label: 'Shell', type: 'select', value: u.shell || '', options: shellOptions.value },
      { key: 'password', label: t('auth.password'), inputType: 'password', help: t('accounts.passwordEditHint') },
      { key: 'locked', label: '', desc: t('accounts.lockAccount'), type: 'checkbox', value: !!u.locked },
    ],
    { submitLabel: t('common.save') },
  );
  if (!result) return;

  const payload = {};
  if (!isSystemUser(u)) {
    if (result.new_name && result.new_name !== u.name) payload.new_name = result.new_name.trim();
    if (result.uid) payload.uid = Number(result.uid);
    if (result.gid) payload.gid = result.gid;
  }
  payload.gecos = result.gecos || '';
  if (result.home) payload.home = result.home;
  if (result.shell) payload.shell = result.shell;
  payload.groups = parseGroups(result.groups);
  if (result.password) payload.password = result.password;
  payload.locked = !!result.locked;

  try {
    await api.put(`/api/accounts/users/${encodeURIComponent(u.name)}`, payload);
    toast.toast(t('accounts.userUpdated', { name: u.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// ── delete ──────────────────────────────────────────────────────────
async function deleteUser(u) {
  const result = await confirm(
    t('accounts.deleteUser'),
    t('accounts.deleteUserConfirm', { name: u.name }),
    [{ key: 'remove_home', label: t('accounts.removeHome') }],
  );
  if (!result || !result.confirmed) return;

  const removeHome = !!result.remove_home;
  try {
    await api.del(`/api/accounts/users/${encodeURIComponent(u.name)}?remove_home=${removeHome}`);
    toast.toast(t('accounts.userDeleted', { name: u.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('accounts.usersTitle') }}</h1>
    <p>{{ t('accounts.usersSubtitle') }}</p>
  </div>
  <div class="toolbar">
    <SearchInput v-model="filter" :placeholder="t('accounts.filterUser')" />
    <span class="text-dim">{{ t('accounts.userCount', { n: filtered.length }) }}</span>
    <button style="margin-left:auto;" @click="addUser">
      <i class="fa-solid fa-plus"></i> {{ t('accounts.addUser') }}
    </button>
  </div>
  <div class="card table-wrap" style="padding:0;">
    <table>
      <thead>
        <tr>
          <th>{{ t('auth.username') }}</th>
          <th>{{ t('accounts.uid') }}</th>
          <th>{{ t('accounts.group') }}</th>
          <th>{{ t('accounts.supplementaryGroups') }}</th>
          <th>{{ t('common.description') }}</th>
          <th>{{ t('accounts.home') }}</th>
          <th>Shell</th>
          <th class="col-actions">{{ t('common.actions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="u in filtered" :key="u.uid">
          <td>
            <strong>{{ u.name }}</strong>
            <span v-if="u.locked" class="badge badge-warn">{{ t('common.locked') }}</span>
          </td>
          <td class="mono">{{ u.uid }}</td>
          <td class="mono">{{ u.group_name || '—' }} <span class="text-dim">({{ u.gid }})</span></td>
          <td>
            <div class="cell-wrap" style="max-width:160px;">
              <template v-if="u.groups && u.groups.length">
                <span v-for="g in u.groups" :key="g" class="badge badge-dim">{{ g }}</span>
              </template>
              <span v-else class="text-dim">—</span>
            </div>
          </td>
          <td>
            <div class="cell-wrap" style="max-width:160px;">{{ u.gecos || '—' }}</div>
          </td>
          <td>
            <div class="mono cell-ellipsis" style="max-width:140px;" :title="u.home">{{ u.home }}</div>
          </td>
          <td>
            <div class="mono cell-ellipsis" style="max-width:120px;" :title="u.shell">{{ u.shell }}</div>
          </td>
          <td class="col-actions">
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="editUser(u)">{{ t('common.edit') }}</button>
              <button class="btn-danger btn-sm" :disabled="isSystemUser(u)" @click="deleteUser(u)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
