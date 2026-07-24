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

const routes = ref([]);
const loading = ref(true);
const refreshing = ref(false);
const error = ref('');

const routesV4 = computed(() => routes.value.filter((r) => r.family === 'ipv4'));
const routesV6 = computed(() => routes.value.filter((r) => r.family === 'ipv6'));

async function load() {
  if (!routes.value.length) loading.value = true;
  refreshing.value = true;
  error.value = '';
  try {
    routes.value = await api.get('/api/network/static-routes');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

function makeFields(route) {
  return [
    {
      key: 'name',
      label: t('common.name'),
      type: 'text',
      value: route?.name || '',
      placeholder: t('staticRoutes.nameHint'),
    },
    {
      key: 'destination',
      label: t('net.destination'),
      type: 'text',
      value: route?.destination || '',
      placeholder: '192.168.1.0/24',
      required: true,
      hint: t('staticRoutes.destinationHint'),
    },
    {
      key: 'gateway',
      label: t('net.gateway'),
      type: 'text',
      value: route?.gateway || '',
      placeholder: '10.0.0.1',
      required: true,
    },
  ];
}

async function doAdd() {
  const result = await formModal(t('staticRoutes.addTitle'), makeFields(), {
    submitLabel: t('common.create'),
    submitHandler: async (r) => {
      await api.post('/api/network/static-routes', {
        destination: r.destination.trim(),
        gateway: r.gateway.trim(),
        name: r.name?.trim() || undefined,
      });
      toast.toast(t('staticRoutes.added'));
      await load();
    },
  });
}

async function doEdit(route) {
  const result = await formModal(t('staticRoutes.editTitle', { name: route.name }), makeFields(route), {
    submitLabel: t('common.save'),
    submitHandler: async (r) => {
      await api.put(`/api/network/static-routes/${route.name}`, {
        destination: r.destination.trim(),
        gateway: r.gateway.trim(),
      });
      toast.toast(t('staticRoutes.updated'));
      await load();
    },
  });
}

async function doDelete(route) {
  const ok = await confirm(
    t('staticRoutes.deleteTitle'),
    t('staticRoutes.deleteConfirm', { name: route.name, dest: route.destination }),
  );
  if (!ok) return;
  try {
    await api.del(`/api/network/static-routes/${route.name}`);
    toast.toast(t('staticRoutes.deleted'));
    await load();
  } catch (err) {
    await alert(t('common.operationFailed'), err.message || t('common.deleteFailed', { msg: '' }));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('staticRoutes.title') }}</h1>
    <p>{{ t('staticRoutes.subtitle') }}</p>
  </div>
  <div class="page-header" style="margin-top:0;">
    <span></span>
    <div class="flex" style="margin-left:auto;">
      <button class="btn-secondary" @click="load" :disabled="refreshing"><i class="fa-solid fa-rotate" :class="{ 'fa-spin': refreshing }"></i> {{ t('common.refresh') }}</button>
      <button @click="doAdd"><i class="fa-solid fa-plus"></i> {{ t('common.add') }}</button>
    </div>
  </div>

  <div v-if="error" class="card" style="padding:1rem;">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="loading" class="card" style="padding:1rem;"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('net.destination') }}</th>
          <th>{{ t('net.gateway') }}</th>
          <th>{{ t('common.type') }}</th>
          <th>{{ t('staticRoutes.family') }}</th>
          <th class="col-actions">{{ t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr class="cron-section-row"><td colspan="6"><div class="cron-section"><span class="cron-section-title">IPv4</span><span class="cron-section-sub text-dim">{{ routesV4.length }}</span></div></td></tr>
          <tr v-if="!routesV4.length"><td colspan="6" class="empty">{{ t('common.noData') }}</td></tr>
          <tr v-for="r in routesV4" :key="r.name">
            <td class="mono">{{ r.name }}</td>
            <td class="mono">{{ r.destination }}</td>
            <td class="mono">{{ r.gateway }}</td>
            <td>{{ r.is_host ? t('staticRoutes.hostRoute') : t('common.network') }}</td>
            <td><span class="badge badge-dim">IPv4</span></td>
            <td>
              <div class="btn-group">
                <button class="btn-secondary btn-sm" @click="doEdit(r)"><i class="fa-solid fa-pen"></i></button>
                <button class="btn-danger btn-sm" @click="doDelete(r)"><i class="fa-solid fa-trash"></i></button>
              </div>
            </td>
          </tr>
          <tr class="cron-section-row"><td colspan="6"><div class="cron-section"><span class="cron-section-title">IPv6</span><span class="cron-section-sub text-dim">{{ routesV6.length }}</span></div></td></tr>
          <tr v-if="!routesV6.length"><td colspan="6" class="empty">{{ t('common.noData') }}</td></tr>
          <tr v-for="r in routesV6" :key="r.name">
            <td class="mono">{{ r.name }}</td>
            <td class="mono">{{ r.destination }}</td>
            <td class="mono">{{ r.gateway }}</td>
            <td>{{ r.is_host ? t('staticRoutes.hostRoute') : t('common.network') }}</td>
            <td><span class="badge badge-dim">IPv6</span></td>
            <td>
              <div class="btn-group">
                <button class="btn-secondary btn-sm" @click="doEdit(r)"><i class="fa-solid fa-pen"></i></button>
                <button class="btn-danger btn-sm" @click="doDelete(r)"><i class="fa-solid fa-trash"></i></button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
