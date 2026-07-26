<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import SmbStatusBar from '../components/shared/SmbStatusBar.vue';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const shares = ref([]);
const loading = ref(true);
const error = ref('');
const smbStatus = ref(null);

const needsInit = computed(() => {
  if (!smbStatus.value) return false;
  const s = smbStatus.value;
  return !s.installed || !s.initialized;
});

const initMessages = computed(() => {
  if (!smbStatus.value) return [];
  const s = smbStatus.value;
  const msgs = [];
  if (!s.installed) msgs.push(t('smb.initMissingPkg'));
  if (!s.initialized) msgs.push(t('smb.initMissingConf'));
  return msgs;
});

async function loadStatus() {
  try {
    smbStatus.value = await api.get('/api/smb/status');
  } catch { smbStatus.value = null; }
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    shares.value = await api.get('/api/smb/shares');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function showForm(existing = null) {
  const isEdit = !!existing;
  const result = await formModal(
    isEdit ? t('smb.editShare') : t('smb.createShare'),
    [
      { key: 'name', label: t('common.name'), value: existing?.name || '', required: true, disabled: isEdit },
      { key: 'comment', label: t('common.description'), value: existing?.comment || '' },
      { key: 'path', label: t('smb.path'), value: existing?.path || '', placeholder: '/zroot/data/share', required: true, picker: 'dir' },
      { key: 'create_mask', label: t('smb.createMask'), value: existing?.create_mask || '0664', half: true },
      { key: 'directory_mask', label: t('smb.directoryMask'), value: existing?.directory_mask || '0775', half: true },
      {
        key: '_flags', label: t('common.options'), type: 'checkbox-group',
        options: [
          { key: 'browseable', label: t('smb.browseable'), value: existing?.browseable ?? true },
          { key: 'writable', label: t('smb.writable'), value: existing?.writable ?? false },
          { key: 'guest_ok', label: t('smb.guestOk'), value: existing?.guest_ok ?? false },
        ],
      },
      { key: 'valid_users', label: t('smb.validUsers'), value: (existing?.valid_users || []).join(' '), placeholder: 'alice bob', help: t('smb.validUsersHint') },
    ],
    { submitLabel: isEdit ? t('common.save') : t('common.create') },
  );
  if (!result) return;

  const body = {
    name: result.name,
    comment: result.comment || '',
    path: result.path,
    browseable: !!result.browseable,
    writable: !!result.writable,
    guest_ok: !!result.guest_ok,
    valid_users: (result.valid_users || '').trim().split(/\s+/).filter(Boolean),
    create_mask: result.create_mask || '0664',
    directory_mask: result.directory_mask || '0775',
  };

  try {
    if (isEdit) {
      await api.put(`/api/smb/shares/${encodeURIComponent(existing.name)}`, body);
      toast.toast(t('smb.shareUpdated', { name: existing.name }));
    } else {
      await api.post('/api/smb/shares', body);
      toast.toast(t('smb.shareCreated', { name: result.name }));
    }
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function deleteShare(share) {
  if (!await confirm(t('smb.deleteShare'), t('smb.deleteShareConfirm', { name: share.name }))) return;
  try {
    await api.del(`/api/smb/shares/${encodeURIComponent(share.name)}`);
    toast.toast(t('smb.shareDeleted', { name: share.name }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(async () => {
  await loadStatus();
  if (!needsInit.value) load();
  else loading.value = false;
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('smb.shares') }}</h1>
    <p>{{ t('smb.sharesSubtitle') }}</p>
    <div style="margin-left:auto;" class="btn-group">
      <button @click="showForm()"><i class="fa-solid fa-plus"></i> {{ t('smb.createShare') }}</button>
      <button class="btn-secondary" @click="load" :disabled="loading"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': loading }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <!-- Not initialized banner -->
  <div v-if="needsInit" class="card" style="border:1px solid var(--warning);padding:24px;text-align:center;">
    <i class="fa-solid fa-triangle-exclamation" style="font-size:32px;color:var(--warning);"></i>
    <h3 style="margin:12px 0 8px;">{{ t('smb.initRequired') }}</h3>
    <p class="text-dim" style="margin-bottom:8px;">{{ t('smb.initRequiredDesc') }}</p>
    <ul v-if="initMessages.length" style="margin:8px auto 16px;display:inline-block;text-align:left;">
      <li v-for="(msg, i) in initMessages" :key="i" style="color:var(--warning);font-size:13px;">{{ msg }}</li>
    </ul>
    <div class="btn-group" style="justify-content:center;">
      <button @click="router.push('/shares/smb/init')">
        <i class="fa-solid fa-rocket"></i> {{ t('smb.initGo') }}
      </button>
    </div>
  </div>

  <template v-else>
    <SmbStatusBar :status="smbStatus" @refresh="loadStatus" />

    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('smb.path') }}</th>
          <th>{{ t('common.description') }}</th>
          <th>{{ t('smb.writable') }}</th>
          <th>{{ t('smb.guestOk') }}</th>
          <th>{{ t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="error"><td colspan="6" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
          <tr v-else-if="loading"><td colspan="6" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
          <tr v-else-if="!shares.length"><td colspan="6" class="empty">{{ t('smb.noShares') }}</td></tr>
          <tr v-for="share in shares" :key="share.name">
            <td class="mono"><strong>{{ share.name }}</strong></td>
            <td class="mono">{{ share.path }}</td>
            <td>{{ share.comment || '—' }}</td>
            <td><span :class="['badge', share.writable ? 'badge-success' : 'badge-dim']">{{ share.writable ? t('common.yes') : t('common.no') }}</span></td>
            <td><span :class="['badge', share.guest_ok ? 'badge-warn' : 'badge-dim']">{{ share.guest_ok ? t('common.yes') : t('common.no') }}</span></td>
            <td>
              <div class="btn-group">
                <button class="btn-secondary btn-sm" @click="showForm(share)"><i class="fa-solid fa-pen"></i></button>
                <button class="btn-danger btn-sm" @click="deleteShare(share)"><i class="fa-solid fa-trash"></i></button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
