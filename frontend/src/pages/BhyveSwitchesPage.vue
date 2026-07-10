<script setup>
import { ref, reactive, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();

const switches = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const showCreate = ref(false);
const form = reactive({
  name: '',
  type: 'standard',
  iface: '',
  vlan: '',
  bridge: '',
  address: '',
  mtu: '',
  private: false,
});

const requiresBridge = computed(() => form.type === 'manual');
const requiresVxlan = computed(() => form.type === 'vxlan');
const supportsStandardOptions = computed(() => form.type === 'standard');
const supportsAddress = computed(() => form.type === 'standard' || form.type === 'vxlan');
const supportsPrivate = computed(() => ['standard', 'manual', 'vxlan'].includes(form.type));

function resetForm() {
  Object.assign(form, {
    name: '', type: 'standard', iface: '', vlan: '', bridge: '',
    address: '', mtu: '', private: false,
  });
}

function openCreate() {
  resetForm();
  showCreate.value = true;
}

function onTypeChange() {
  if (!supportsStandardOptions.value) {
    form.iface = '';
    form.mtu = '';
  }
  if (!supportsAddress.value) form.address = '';
  if (!supportsPrivate.value) form.private = false;
  if (!requiresBridge.value) form.bridge = '';
}

async function load() {
  if (!switches.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    switches.value = await api.get('/api/bhyve/switches');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function deleteSwitch(sw) {
  if (!await confirm(t('bhyve.switchDeleteTitle'), t('bhyve.switchDeleteConfirm', { name: sw.name }))) return;
  try {
    await api.del(`/api/bhyve/switches/${encodeURIComponent(sw.name)}`);
    toast.toast(t('bhyve.switchDeleted', { name: sw.name }));
    await load();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

async function submitCreate() {
  const body = {
    name: form.name.trim(),
    type: form.type,
    private: form.private,
  };
  if (form.iface.trim()) body.iface = form.iface.trim();
  if (form.bridge.trim()) body.bridge = form.bridge.trim();
  if (form.address.trim()) body.address = form.address.trim();
  if (form.vlan !== '') body.vlan = Number(form.vlan);
  if (form.mtu !== '') body.mtu = Number(form.mtu);

  try {
    await api.post('/api/bhyve/switches', body);
    toast.toast(t('bhyve.switchCreated', { name: body.name }));
    showCreate.value = false;
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.tabSwitches') }}</h1>
    <p>{{ t('bhyve.switchSubtitle') }}</p>
  </div>

  <div class="toolbar">
    <span class="text-dim">{{ t('bhyve.switchCount', { n: switches.length }) }}</span>
    <div class="flex btn-group">
      <button @click="openCreate"><i class="fa-solid fa-plus"></i> {{ t('bhyve.createSwitch') }}</button>
      <button @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div class="card" style="padding:0;">
    <table>
      <thead><tr>
        <th>{{ t('common.name') }}</th>
        <th>{{ t('common.type') }}</th>
        <th>{{ t('bhyve.interface') }}</th>
        <th>{{ t('bhyve.address') }}</th>
        <th>{{ t('bhyve.private') }}</th>
        <th>MTU</th>
        <th>VLAN</th>
        <th>{{ t('bhyve.ports') }}</th>
        <th>{{ t('common.actions') }}</th>
      </tr></thead>
      <tbody>
        <tr v-if="error"><td colspan="9" class="empty">{{ t('common.loadFailed', { msg: error }) }}</td></tr>
        <tr v-else-if="loading"><td colspan="9" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td></tr>
        <tr v-else-if="!switches.length"><td colspan="9" class="empty">{{ t('bhyve.noSwitches') }}</td></tr>
        <tr v-for="sw in switches" :key="sw.name" class="row-clickable" @click="router.push(`/bhyve/switches/${sw.name}`)">
          <td class="mono"><strong>{{ sw.name }}</strong></td>
          <td>{{ sw.type }}</td>
          <td class="mono">{{ sw.iface }}</td>
          <td class="mono">{{ sw.address || '—' }}</td>
          <td>
            <span :class="['badge', sw.private ? 'badge-warn' : 'badge-dim']">{{ sw.private ? t('common.yes') : t('common.no') }}</span>
          </td>
          <td class="mono">{{ sw.mtu || '—' }}</td>
          <td class="mono">{{ sw.vlan || '—' }}</td>
          <td class="mono">{{ sw.ports.length ? sw.ports.join(', ') : '—' }}</td>
          <td>
            <button class="btn-danger btn-sm" @click.stop="deleteSwitch(sw)">{{ t('common.delete') }}</button>
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <div v-if="showCreate" class="modal-overlay">
    <div class="modal" style="max-width:600px;">
      <h3>{{ t('bhyve.createSwitch') }}</h3>
      <form @submit.prevent="submitCreate">
        <div class="field">
          <label>{{ t('common.name') }} <span style="color:var(--danger)">*</span></label>
          <input v-model="form.name" required :placeholder="t('bhyve.switchNamePlaceholder')" />
        </div>
        <div class="field">
          <label>{{ t('common.type') }} <span style="color:var(--danger)">*</span></label>
          <select v-model="form.type" @change="onTypeChange">
            <option value="standard">{{ t('bhyve.switchTypeStandard') }}</option>
            <option value="manual">{{ t('bhyve.switchTypeManual') }}</option>
            <option value="netgraph">Netgraph</option>
            <option value="vale">VALE</option>
            <option value="vxlan">VXLAN</option>
          </select>
        </div>

        <div v-if="requiresBridge" class="field">
          <label>{{ t('bhyve.bridge') }} <span style="color:var(--danger)">*</span></label>
          <input v-model="form.bridge" required placeholder="bridge0" />
        </div>

        <template v-if="supportsStandardOptions || requiresVxlan">
          <div class="field">
            <label>{{ t('bhyve.interface') }}<span v-if="requiresVxlan" style="color:var(--danger)"> *</span></label>
            <input v-model="form.iface" :required="requiresVxlan" placeholder="em0" />
          </div>
          <div class="field">
            <label>VLAN{{ requiresVxlan ? ' ID' : '' }}<span v-if="requiresVxlan" style="color:var(--danger)"> *</span></label>
            <input v-model.number="form.vlan" type="number" min="0" max="4094" :required="requiresVxlan" placeholder="100" />
          </div>
        </template>

        <div v-if="supportsAddress" class="field">
          <label>{{ t('bhyve.address') }}</label>
          <input v-model="form.address" placeholder="192.168.10.1/24" />
        </div>
        <div v-if="supportsStandardOptions" class="field">
          <label>MTU</label>
          <input v-model.number="form.mtu" type="number" min="100" max="9000" placeholder="1500" />
        </div>
        <label v-if="supportsPrivate" class="flex" style="gap:6px;cursor:pointer;align-items:center;margin:12px 0;">
          <input v-model="form.private" type="checkbox" />
          {{ t('bhyve.private') }}
        </label>

        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showCreate = false">{{ t('common.cancel') }}</button>
          <button type="submit">{{ t('common.create') }}</button>
        </div>
      </form>
    </div>
  </div>
</template>
