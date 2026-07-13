<script setup>
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import PoolManageModal from '../components/ui/PoolManageModal.vue';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();
const pools = ref(null);
const error = ref('');
const showCreate = ref(false);
const refreshing = ref(false);

// Import dialog state
const showImport = ref(false);
const importable = ref([]);
const importing = ref(false);
const includeDestroyed = ref(false);

function barClass(pct) {
  return pct > 80 ? 'bar-swap' : 'bar-mem';
}

async function load() {
  refreshing.value = true;
  try {
    pools.value = await api.get('/api/zfs/pools');
    error.value = '';
  } catch (err) {
    error.value = err.message || '';
  } finally {
    refreshing.value = false;
  }
}

async function destroyPool(name) {
  const result = await confirm(
    t('zfs.poolDestroy'),
    t('zfs.poolDestroyConfirm', { name }),
    [{ key: 'force', label: t('zfs.poolDestroyForce'), checked: false }],
  );
  if (!result || !result.confirmed) return;
  try {
    await api.del(`/api/zfs/pools/${name}?force=${result.force ? 'true' : 'false'}`);
    toast.toast(t('zfs.poolDestroyed'));
    await load();
  } catch (e) {
    await alert(t('zfs.poolDestroyFailed'), e.message || t('common.operationFailed'));
  }
}

async function exportPool(name) {
  const result = await confirm(
    t('zfs.poolExport'),
    t('zfs.poolExportConfirm', { name }),
  );
  if (!result) return;
  try {
    await api.post(`/api/zfs/pools/${name}/export`);
    toast.toast(t('zfs.poolExported', { name }));
    await load();
  } catch (e) {
    await alert(t('zfs.poolExportFailed'), e.message || t('common.operationFailed'));
  }
}

async function openImport() {
  showImport.value = true;
  includeDestroyed.value = false;
  await fetchImportable();
}

async function fetchImportable() {
  importable.value = [];
  try {
    importable.value = await api.get(`/api/zfs/pools/importable?include_destroyed=${includeDestroyed.value ? 'true' : 'false'}`);
  } catch (e) {
    await alert(t('zfs.poolImportFailed'), e.message || t('common.operationFailed'));
  }
}

async function doImport(poolName, isDestroyed) {
  if (isDestroyed) {
    const ok = await confirm(t('zfs.poolImportWarningTitle'), t('zfs.poolImportWarning'));
    if (!ok) return;
  }
  const result = await formModal(t('zfs.poolImportTitle', { name: poolName }), [
    { key: 'altroot', label: t('zfs.poolImportAltroot'), placeholder: t('zfs.poolImportAltrootHint') },
  ]);
  if (!result) return;
  importing.value = true;
  try {
    await api.post('/api/zfs/pools/import', {
      name: poolName,
      altroot: result.altroot || undefined,
      destroyed: isDestroyed || undefined,
    });
    toast.toast(t('zfs.poolImported', { name: poolName }));
    showImport.value = false;
    await load();
  } catch (e) {
    await alert(t('zfs.poolImportFailed'), e.message || t('common.operationFailed'));
  } finally {
    importing.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>Zpool</h1>
    <p>{{ t('zfs.poolsSubtitle') }}</p>
    <div class="flex" style="margin-left:auto;">
      <button class="btn-secondary" @click="openImport"><i class="fa-solid fa-download"></i> {{ t('common.import') }}</button>
      <button @click="showCreate = true"><i class="fa-solid fa-plus"></i> {{ t('common.create') }}</button>
      <button class="btn-secondary" @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!pools" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  <template v-else>
    <div v-if="!pools.length" class="empty">{{ t('zfs.noPools') }}</div>
    <div v-for="p in pools" :key="p.name" class="card pool-card" style="cursor:pointer;" @click="router.push(`/zfs/pools/${p.name}`)">
      <div class="flex" style="justify-content:space-between;">
        <div>
          <span :class="['badge', p.health === 'ONLINE' ? 'badge-success' : 'badge-danger']">{{ p.health }}</span>
          <strong style="font-size:18px;margin-left:8px;">{{ p.name }}</strong>
        </div>
        <div class="btn-group">
          <button class="btn-secondary btn-sm" @click.stop="exportPool(p.name)">{{ t('zfs.poolExport') }}</button>
          <button class="btn-danger btn-sm" @click.stop="destroyPool(p.name)">{{ t('zfs.poolDestroy') }}</button>
        </div>
      </div>
      <div class="stat-row" style="margin-top:12px;">
        <span>{{ t('common.capacity') }}: <strong>{{ fmtBytes(p.size) }}</strong></span>
        <span>{{ t('common.used') }}: {{ fmtBytes(p.allocated) }}</span>
        <span>{{ t('common.free') }}: {{ fmtBytes(p.free) }}</span>
        <span>{{ t('common.frag') }}: {{ p.fragmentation_pct.toFixed(0) }}%</span>
        <span>{{ t('common.dedup') }}: {{ p.dedup.toFixed(2) }}x</span>
      </div>
      <div class="bar-wrap" style="margin-top:10px;">
        <div :class="['bar', barClass(p.capacity_pct)]" :style="{ width: p.capacity_pct + '%' }"></div>
      </div>
    </div>
  </template>

  <PoolManageModal
    :show="showCreate"
    mode="create"
    @close="showCreate = false"
    @success="showCreate = false; load()"
  />

  <!-- Import dialog -->
  <div v-if="showImport" class="modal-overlay" @click.self="showImport = false">
    <div class="modal" style="max-width:560px;">
      <h3>{{ t('zfs.poolImport') }}</h3>
      <div class="flex" style="justify-content:space-between;align-items:center;margin-bottom:12px;">
        <span class="text-dim" style="font-size:13px;">{{ t('zfs.poolImportHint') }}</span>
        <label class="checkbox-row">
          <input type="checkbox" v-model="includeDestroyed" @change="fetchImportable" />
          <span style="font-size:13px;">{{ t('zfs.poolImportDestroyed') }}</span>
        </label>
      </div>
      <div v-if="!importable.length" class="empty">{{ t('zfs.noImportablePools') }}</div>
      <div v-for="p in importable" :key="p.id" class="card" style="padding:12px;margin-bottom:8px;">
        <div class="flex" style="justify-content:space-between;align-items:center;">
          <div>
            <span class="badge badge-dim">{{ p.state }}</span>
            <strong style="font-size:16px;margin-left:8px;">{{ p.name }}</strong>
            <span class="text-dim" style="font-size:12px;margin-left:8px;">{{ fmtBytes(p.size) }}</span>
          </div>
          <button :disabled="importing" @click="doImport(p.name, includeDestroyed)">{{ t('common.import') }}</button>
        </div>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" @click="showImport = false">{{ t('common.close') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.btn-sm { padding: 4px 12px; font-size: 12px; }
</style>
