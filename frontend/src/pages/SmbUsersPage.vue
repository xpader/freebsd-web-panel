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

const users = ref([]);
const loading = ref(true);
const error = ref('');

async function load() {
  loading.value = true;
  error.value = '';
  try {
    users.value = await api.get('/api/smb/users');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function addUser() {
  let sysUsers;
  try {
    sysUsers = await api.get('/api/smb/sysusers');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  if (!sysUsers.length) {
    await alert(t('smb.noSysUsers'), t('smb.noSysUsersDesc'));
    return;
  }
  const userOptions = sysUsers.map(u => ({ value: u.username, label: `${u.username} (${u.gecos || u.uid})` }));
  const result = await formModal(
    t('smb.addUser'),
    [
      { key: 'username', label: t('common.name'), type: 'select', options: userOptions, required: true },
      { key: 'password', label: t('common.password'), type: 'password', required: true },
    ],
    { submitLabel: t('common.create') },
  );
  if (!result) return;

  try {
    await api.post('/api/smb/users', { username: result.username, password: result.password });
    toast.toast(t('smb.userCreated', { name: result.username }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function changePassword(user) {
  const result = await formModal(
    t('smb.changePassword', { name: user.username }),
    [
      { key: 'password', label: t('common.password'), type: 'password', required: true },
    ],
    { submitLabel: t('common.save') },
  );
  if (!result) return;

  try {
    await api.put(`/api/smb/users/${encodeURIComponent(user.username)}/password`, { password: result.password });
    toast.toast(t('smb.passwordChanged', { name: user.username }));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function deleteUser(user) {
  if (!await confirm(t('smb.deleteUser'), t('smb.deleteUserConfirm', { name: user.username }))) return;
  try {
    await api.del(`/api/smb/users/${encodeURIComponent(user.username)}`);
    toast.toast(t('smb.userDeleted', { name: user.username }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('smb.users') }}</h1>
    <p>{{ t('smb.usersSubtitle') }}</p>
    <div style="margin-left:auto;" class="btn-group">
      <button @click="addUser"><i class="fa-solid fa-plus"></i> {{ t('smb.addUser') }}</button>
      <button class="btn-secondary" @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th>
        <th>UID</th>
        <th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="3" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="3" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!users.length"><td colspan="3" class="empty">{{ t('smb.noUsers') }}</td></tr>
        <tr v-for="user in users" :key="user.username">
          <td class="mono"><strong>{{ user.username }}</strong></td>
          <td class="mono">{{ user.uid }}</td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="changePassword(user)"><i class="fa-solid fa-key"></i></button>
              <button class="btn-danger btn-sm" @click="deleteUser(user)"><i class="fa-solid fa-trash"></i></button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
