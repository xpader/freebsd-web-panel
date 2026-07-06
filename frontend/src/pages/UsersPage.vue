<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtTime } from '../lib/format.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const users = ref(null);
const error = ref('');
const showAddModal = ref(false);
const showEditModal = ref(false);
const editUser = ref(null);
const formUsername = ref('');
const formPassword = ref('');

async function load() {
  try {
    users.value = await api.get('/api/users');
  } catch (err) {
    error.value = err.message || '';
  }
}

function openAdd() {
  formUsername.value = '';
  formPassword.value = '';
  showAddModal.value = true;
}

function openEdit(user) {
  editUser.value = user;
  formUsername.value = user.username;
  formPassword.value = '';
  showEditModal.value = true;
}

async function doAdd() {
  try {
    await api.post('/api/users', { username: formUsername.value, password: formPassword.value });
    toast.toast(t('users.created'));
    showAddModal.value = false;
    await load();
  } catch (err) {
    await alert(t('common.operationFailed'), err.message || t('common.operationFailed'));
  }
}

async function doEdit() {
  try {
    await api.put(`/api/users/${editUser.value.id}`, { password: formPassword.value });
    toast.toast(t('users.pwdUpdated'));
    showEditModal.value = false;
  } catch (err) {
    await alert(t('common.operationFailed'), err.message || t('common.operationFailed'));
  }
}

async function doDelete(user) {
  if (!await confirm(t('common.delete'), t('users.deleteConfirm', { name: user.username }))) return;
  try {
    await api.del(`/api/users/${user.id}`);
    toast.toast(t('users.deleted'));
    await load();
  } catch (err) {
    await alert(t('common.deleteFailed'), err.message || t('common.deleteFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('users.title') }}</h1>
    <p>{{ t('users.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <div></div>
    <button @click="openAdd">{{ t('users.add') }}</button>
  </div>
  <div class="card" style="padding:0;">
    <table>
      <thead><tr><th>ID</th><th>{{ t('auth.username') }}</th><th>{{ t('users.role') }}</th><th>{{ t('common.createdAt') }}</th><th>{{ t('users.lastLogin') }}</th><th>{{ t('common.actions') }}</th></tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="!users"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!users.length"><td colspan="6" class="empty">{{ t('users.noUsers') }}</td></tr>
        <tr v-for="u in users" :key="u.id">
          <td class="mono">{{ u.id }}</td>
          <td><strong>{{ u.username }}</strong></td>
          <td><span class="badge badge-success">{{ u.role }}</span></td>
          <td class="text-dim mono">{{ fmtTime(u.created_at) }}</td>
          <td class="text-dim mono">{{ u.last_login ? fmtTime(u.last_login) : '—' }}</td>
          <td>
            <div class="btn-group">
              <button class="btn-secondary btn-sm" @click="openEdit(u)">{{ t('users.changePwd') }}</button>
              <button class="btn-danger btn-sm" @click="doDelete(u)">{{ t('common.delete') }}</button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Add modal -->
  <div v-if="showAddModal" class="modal-overlay">
    <div class="modal">
      <h3>{{ t('users.add') }}</h3>
      <form @submit.prevent="doAdd">
        <div class="field">
          <label>{{ t('auth.username') }}</label>
          <input type="text" v-model="formUsername" required />
        </div>
        <div class="field">
          <label>{{ t('auth.passwordMin') }}</label>
          <input type="password" v-model="formPassword" required minlength="6" />
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showAddModal = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.ok') }}</button>
        </div>
      </form>
    </div>
  </div>

  <!-- Edit password modal -->
  <div v-if="showEditModal" class="modal-overlay">
    <div class="modal">
      <h3>{{ t('users.editPwdTitle', { name: editUser.username }) }}</h3>
      <form @submit.prevent="doEdit">
        <div class="field">
          <label>{{ t('auth.passwordMin') }}</label>
          <input type="password" v-model="formPassword" required minlength="6" />
        </div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showEditModal = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.ok') }}</button>
        </div>
      </form>
    </div>
  </div>
</template>
