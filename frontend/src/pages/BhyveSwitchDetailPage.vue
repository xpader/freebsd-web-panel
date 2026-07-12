<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import BackButton from '../components/ui/BackButton.vue';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const sw = ref(null);       // from vm switch list
const detail = ref(null);    // from vm switch info
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');
const busy = ref(false);

const physicalPorts = computed(() => {
  if (!detail.value?.fields) return [];
  const raw = detail.value.fields['physical-ports'];
  if (!raw || raw === '-') return [];
  return raw.split(/\s+/).filter(Boolean);
});

async function load() {
  if (!sw.value) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    const [switches, d] = await Promise.all([
      api.get('/api/bhyve/switches'),
      api.get(`/api/bhyve/switches/${encodeURIComponent(name)}`),
    ]);
    sw.value = switches.find(s => s.name === name) || null;
    detail.value = d;
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function togglePrivate() {
  if (busy.value) return;
  busy.value = true;
  const target = !sw.value?.private;
  try {
    await api.put(`/api/bhyve/switches/${encodeURIComponent(name)}/private`, { private: target });
    toast.toast(t('bhyve.switchPrivateSet', { state: t(target ? 'common.on' : 'common.off') }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

async function editVlan() {
  if (busy.value) return;
  const current = sw.value?.vlan ? Number(sw.value.vlan) : '';
  const result = await formModal(t('bhyve.switchVlanTitle'), [
    { key: 'vlan', label: 'VLAN ID', type: 'number', placeholder: '0 = clear', value: current },
  ], t('common.save'));
  if (!result) return;
  const vlan = Number(result.vlan) || 0;
  busy.value = true;
  try {
    await api.put(`/api/bhyve/switches/${encodeURIComponent(name)}/vlan`, { vlan });
    toast.toast(t('bhyve.switchVlanSet', { vlan }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

async function editAddress() {
  if (busy.value) return;
  const current = sw.value?.address || '';
  const result = await formModal(t('bhyve.switchAddressTitle'), [
    { key: 'address', label: t('bhyve.address'), placeholder: '192.168.1.1/24 (empty = clear)', value: current },
  ], t('common.save'));
  if (!result) return;
  const address = result.address?.trim() || null;
  busy.value = true;
  try {
    await api.put(`/api/bhyve/switches/${encodeURIComponent(name)}/address`, { address });
    toast.toast(address ? t('bhyve.switchAddressSet', { address }) : t('bhyve.switchAddressCleared'));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

async function addPort() {
  if (busy.value) return;
  const result = await formModal(t('bhyve.switchAddPortTitle'), [
    { key: 'interface', label: t('bhyve.interface'), placeholder: 'em0', required: true },
  ], t('common.add'));
  if (!result) return;
  const iface = result.interface.trim();
  busy.value = true;
  try {
    await api.post(`/api/bhyve/switches/${encodeURIComponent(name)}/ports`, { interface: iface });
    toast.toast(t('bhyve.switchPortAdded', { iface }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

async function removePort(iface) {
  if (busy.value) return;
  if (!await confirm(t('bhyve.switchRemovePortTitle'), t('bhyve.switchRemovePortConfirm', { iface }))) return;
  busy.value = true;
  try {
    await api.del(`/api/bhyve/switches/${encodeURIComponent(name)}/ports/${encodeURIComponent(iface)}`);
    toast.toast(t('bhyve.switchPortRemoved', { iface }));
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    busy.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/bhyve/switches" />
      <h1>{{ name }}</h1>
    </div>
    <div class="btn-group">
      <button class="btn-secondary" @click="load" :disabled="!detail || refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div v-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  <div v-else-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!detail" class="empty">{{ t('bhyve.noSwitches') }}</div>

  <template v-else>
    <!-- Properties -->
    <div class="card">
      <h3>{{ t('common.overview') }}</h3>
      <table>
    <tbody>
      <tr>
        <td>{{ t('common.type') }}</td>
        <td class="mono">{{ detail.fields.type || '—' }}</td>
        <td></td>
      </tr>
      <tr>
        <td>{{ t('bhyve.interface') }}</td>
        <td class="mono">{{ detail.fields.ident || sw?.iface || '—' }}</td>
        <td></td>
      </tr>
      <tr>
        <td>{{ t('bhyve.address') }}</td>
        <td class="mono">{{ sw?.address || '—' }}</td>
        <td class="col-actions">
          <button class="btn-secondary btn-sm" @click="editAddress" :disabled="busy"><i class="fa-solid fa-pen"></i> {{ t('common.edit') }}</button>
        </td>
      </tr>
      <tr>
        <td>VLAN</td>
        <td class="mono">{{ sw?.vlan || '—' }}</td>
        <td class="col-actions">
          <button class="btn-secondary btn-sm" @click="editVlan" :disabled="busy"><i class="fa-solid fa-pen"></i> {{ t('common.edit') }}</button>
        </td>
      </tr>
      <tr>
        <td>{{ t('bhyve.private') }}</td>
        <td>
          <label class="toggle-switch" :class="{ disabled: busy }">
            <input type="checkbox" :checked="sw?.private" :disabled="busy" @change="togglePrivate" />
            <span class="toggle-slider"></span>
          </label>
        </td>
        <td></td>
      </tr>
      <tr>
        <td>MTU</td>
        <td class="mono">{{ sw?.mtu || '—' }}</td>
        <td></td>
      </tr>
    </tbody>
      </table>
    </div>

    <!-- Physical Ports -->
    <div class="card">
      <h3>{{ t('bhyve.physicalPorts') }}</h3>
      <div v-if="!physicalPorts.length" class="empty">{{ t('bhyve.noPhysicalPorts') }}</div>
      <table v-else>
    <thead><tr>
      <th>{{ t('bhyve.interface') }}</th>
      <th>{{ t('common.actions') }}</th>
    </tr></thead>
    <tbody>
      <tr v-for="port in physicalPorts" :key="port">
        <td class="mono">{{ port }}</td>
        <td>
      <button class="btn-danger btn-sm" @click="removePort(port)" :disabled="busy"><i class="fa-solid fa-trash"></i> {{ t('common.remove') }}</button>
        </td>
      </tr>
    </tbody>
      </table>
      <div style="margin-top:12px;">
    <button @click="addPort" :disabled="busy"><i class="fa-solid fa-plus"></i> {{ t('bhyve.switchAddPort') }}</button>
      </div>
    </div>

    <!-- Virtual Ports (connected VMs) -->
    <div class="card">
      <h3>{{ t('bhyve.virtualPorts') }}</h3>
      <div v-if="!detail.virtual_ports.length" class="empty">{{ t('bhyve.noVirtualPorts') }}</div>
      <table v-else>
    <thead><tr>
      <th>{{ t('common.device') }}</th>
      <th>VM</th>
    </tr></thead>
    <tbody>
      <tr v-for="vp in detail.virtual_ports" :key="vp.device">
        <td class="mono">{{ vp.device }}</td>
        <td class="mono">{{ vp.vm }}</td>
      </tr>
    </tbody>
      </table>
    </div>

    <!-- Traffic Stats -->
    <div class="card" v-if="detail.fields['bytes-in'] || detail.fields['bytes-out']">
      <h3>{{ t('bhyve.trafficStats') }}</h3>
      <table>
    <tbody>
      <tr>
        <td>{{ t('bhyve.bytesIn') }}</td>
        <td class="mono">{{ detail.fields['bytes-in'] || '—' }}</td>
      </tr>
      <tr>
        <td>{{ t('bhyve.bytesOut') }}</td>
        <td class="mono">{{ detail.fields['bytes-out'] || '—' }}</td>
      </tr>
    </tbody>
      </table>
    </div>
  </template>
</template>

<style scoped>
.col-actions {
  text-align: right;
  white-space: nowrap;
  width: 1%;
}

.toggle-switch {
  position: relative;
  display: inline-block;
  width: 42px;
  height: 24px;
  flex-shrink: 0;
}
.toggle-switch input {
  opacity: 0;
  width: 0;
  height: 0;
}
.toggle-slider {
  position: absolute;
  cursor: pointer;
  inset: 0;
  background: var(--bg-elev2);
  border-radius: 24px;
  transition: 0.2s;
}
.toggle-slider::before {
  content: '';
  position: absolute;
  width: 18px;
  height: 18px;
  left: 3px;
  bottom: 3px;
  background: #ccc;
  border-radius: 50%;
  transition: 0.2s;
}
.toggle-switch input:checked + .toggle-slider {
  background: var(--accent);
}
.toggle-switch input:checked + .toggle-slider::before {
  transform: translateX(18px);
  background: #fff;
}
.toggle-switch.disabled .toggle-slider {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
