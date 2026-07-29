<script setup>
import { ref, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import BackButton from '../components/ui/BackButton.vue';
import PoolManageModal from '../components/ui/PoolManageModal.vue';
import ProgressBar from '../components/ui/ProgressBar.vue';

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const name = route.params.name;
const info = ref(null);
const error = ref('');
const showAddVdev = ref(false);
const refreshing = ref(false);

function healthBadge(health) {
  if (health === 'ONLINE') return 'badge-success';
  if (health === 'DEGRADED') return 'badge-warn';
  return 'badge-danger';
}


async function load() {
  refreshing.value = true;
  try {
    info.value = await api.get(`/api/zfs/pools/${name}`);
    error.value = '';
  } catch (err) {
    error.value = err.message || '';
  } finally {
    refreshing.value = false;
  }
}

async function scrub() {
  try {
    await api.post(`/api/zfs/pools/${name}/scrub`);
    toast.toast(t('zfs.scrubStarted', { name }));
    await load();
  } catch (e) {
    await alert(t('zfs.scrubFailed'), e.message || t('common.operationFailed'));
  }
}

async function scrubStop() {
  try {
    await api.post(`/api/zfs/pools/${name}/scrub/stop`);
    toast.toast(t('zfs.scrubStopped', { name }));
    await load();
  } catch (e) {
    await alert(t('zfs.scrubFailed'), e.message || t('common.operationFailed'));
  }
}

async function destroyPool() {
  const result = await confirm(
    t('zfs.poolDestroy'),
    t('zfs.poolDestroyConfirm', { name }),
    [{ key: 'force', label: t('zfs.poolDestroyForce'), checked: false }],
  );
  if (!result || !result.confirmed) return;
  try {
    await api.del(`/api/zfs/pools/${name}?force=${result.force ? 'true' : 'false'}`);
    toast.toast(t('zfs.poolDestroyed'));
    router.push('/zfs/pools');
  } catch (e) {
    await alert(t('zfs.poolDestroyFailed'), e.message || t('common.operationFailed'));
  }
}

async function exportPool() {
  const result = await confirm(
    t('zfs.poolExport'),
    t('zfs.poolExportConfirm', { name }),
  );
  if (!result) return;
  try {
    await api.post(`/api/zfs/pools/${name}/export`);
    toast.toast(t('zfs.poolExported', { name }));
    router.push('/zfs/pools');
  } catch (e) {
    await alert(t('zfs.poolExportFailed'), e.message || t('common.operationFailed'));
  }
}

async function getAvailableDiskOptions() {
  try {
    const disks = await api.get('/api/zfs/pools/available-disks');
    return disks
      .filter(d => !d.in_use)
      .map(d => ({
        value: d.name,
        label: `${d.name} (${fmtBytes(d.size_bytes)})`,
      }));
  } catch {
    return [];
  }
}

async function attachDisk(device) {
  const opts = await getAvailableDiskOptions();
  if (opts.length === 0) {
    await alert(t('zfs.attachDisk'), t('zfs.noAvailableDisks'));
    return;
  }
  const result = await formModal(t('zfs.attachTitle', { device }), [
    {
      key: 'new_device',
      label: t('common.device'),
      type: 'select',
      options: opts,
      required: true,
    },
  ]);
  if (!result || !result.new_device) return;
  try {
    await api.post(`/api/zfs/pools/${name}/attach`, {
      device,
      new_device: result.new_device,
    });
    toast.toast(t('zfs.attachDisk') + ': ' + result.new_device);
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function detachDisk(device) {
  const result = await confirm(t('zfs.detachTitle'), t('zfs.detachConfirm', { device }));
  if (!result) return;
  try {
    await api.post(`/api/zfs/pools/${name}/detach?device=${encodeURIComponent(device)}`);
    toast.toast(t('zfs.detachDisk') + ': ' + device);
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function replaceDisk(device) {
  const opts = await getAvailableDiskOptions();
  if (opts.length === 0) {
    await alert(t('zfs.replaceDisk'), t('zfs.noAvailableDisks'));
    return;
  }
  const result = await formModal(t('zfs.replaceTitle', { device }), [
    {
      key: 'new_device',
      label: t('zfs.replaceDisk'),
      type: 'select',
      options: opts,
      required: true,
    },
  ]);
  if (!result || !result.new_device) return;
  try {
    await api.post(`/api/zfs/pools/${name}/replace`, {
      old_device: device,
      new_device: result.new_device,
    });
    toast.toast(t('zfs.replaceDisk') + ': ' + result.new_device);
    await load();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <BackButton href="#/zfs/pools" />
    <h1>{{ t('zfs.poolTitle', { name }) }}</h1>
    <p>{{ t('zfs.poolDetailSubtitle') }}</p>
    <button class="btn-secondary" style="margin-left:auto;" @click="load" :disabled="refreshing"><i :class="['fa-solid fa-rotate-right', { 'fa-spin': refreshing }]"></i> {{ t('common.refresh') }}</button>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!info" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <div class="stat-grid">
      <div class="card"><div class="card-title">{{ t('zfs.state') }}</div><div class="card-value sm"><span :class="['badge', healthBadge(info.health)]">{{ info.health }}</span></div></div>
      <div class="card"><div class="card-title">{{ t('zfs.totalSize') }}</div><div class="card-value sm">{{ fmtBytes(info.size) }}</div></div>
      <div class="card"><div class="card-title">{{ t('zfs.allocated') }}</div><div class="card-value sm">{{ fmtBytes(info.allocated) }} ({{ (info.capacity_pct || 0).toFixed(0) }}%)</div></div>
      <div class="card"><div class="card-title">{{ t('common.free') }}</div><div class="card-value sm">{{ fmtBytes(info.free) }}</div></div>
      <div class="card"><div class="card-title">{{ t('common.frag') }}</div><div class="card-value sm"><span :class="['badge', info.fragmentation_pct > 70 ? 'badge-danger' : info.fragmentation_pct > 50 ? 'badge-warn' : 'badge-success']">{{ info.fragmentation_pct.toFixed(0) }}%</span></div></div>
      <div class="card"><div class="card-title">{{ t('common.dedup') }}</div><div class="card-value sm">{{ info.dedup.toFixed(2) }}x</div></div>
    </div>

    <div class="card">
      <div class="card-title">{{ t('zfs.capacityUsage') }}</div>
      <ProgressBar :pct="info.capacity_pct || 0" variant="auto" style="height:16px;" />
      <div class="text-dim" style="font-size:12px;margin-top:6px;">{{ fmtBytes(info.allocated) }} / {{ fmtBytes(info.size) }} ({{ (info.capacity_pct || 0).toFixed(1) }}%)</div>
    </div>

    <div v-if="info.scan" class="card">
      <div class="card-title">{{ t('zfs.scrubStatus') }}</div>
      <pre style="font-size:13px;margin:0;white-space:pre-wrap;font-family:var(--font-mono,monospace);line-height:1.6;">{{ info.scan }}</pre>
    </div>

    <div v-if="info.expand" class="card">
      <div class="card-title">{{ t('zfs.expandStatus') }}</div>
      <pre style="font-size:13px;margin:0;white-space:pre-wrap;font-family:var(--font-mono,monospace);line-height:1.6;">{{ info.expand }}</pre>
    </div>

    <!-- VDEV tree with per-disk operations -->
    <div class="card">
      <div class="flex" style="justify-content:space-between;align-items:center;">
        <div class="card-title">{{ t('zfs.vdevTree') }}</div>
        <button class="btn-secondary btn-sm" @click="showAddVdev = true">+ {{ t('common.add') }} VDEV</button>
      </div>
      <template v-for="(v, vi) in (info.vdevs || [])" :key="vi">
        <div class="vdev-node">
          <div class="vdev-node-row flex">
            <span class="mono" style="font-size:13px;font-weight:600;">{{ v.name }}</span>
            <span class="badge badge-dim" style="margin:0 8px;font-size:10px;">{{ v.name.startsWith('mirror') ? t('zfs.vdevMirror') : v.name.startsWith('raidz') ? t('zfs.vdevRaidz') : !v.children.length ? t('zfs.vdevDisk') : t('zfs.vdevGeneric') }}</span>
            <span :class="['badge', v.state === 'ONLINE' ? 'badge-success' : v.state === 'DEGRADED' ? 'badge-warn' : 'badge-danger']" style="font-size:10px;">{{ v.state }}</span>
            <span v-if="v.read_errors > 0 || v.write_errors > 0 || v.checksum_errors > 0" class="badge badge-danger" style="font-size:10px;">R:{{ v.read_errors }} W:{{ v.write_errors }} C:{{ v.checksum_errors }}</span>
            <!-- Attach: standalone disk (→mirror), or RAID-Z/mirror vdev (expand) -->
            <div v-if="!v.children.length || v.name.startsWith('raidz') || v.name.startsWith('mirror')" class="btn-group" style="margin-left:auto;">
              <button class="btn-secondary btn-tiny" @click="attachDisk(v.name)">{{ t('zfs.attachDisk') }}</button>
              <button v-if="!v.children.length" class="btn-secondary btn-tiny" @click="replaceDisk(v.name)">{{ t('zfs.replaceDisk') }}</button>
            </div>
          </div>
          <template v-for="(c, ci) in v.children" :key="ci">
            <div class="vdev-node" style="margin-left:24px;">
              <div class="vdev-node-row flex">
                <span class="mono" style="font-size:13px;">{{ c.name }}</span>
                <span class="badge badge-dim" style="margin:0 8px;font-size:10px;">{{ t('zfs.vdevDisk') }}</span>
                <span :class="['badge', c.state === 'ONLINE' ? 'badge-success' : c.state === 'DEGRADED' ? 'badge-warn' : 'badge-danger']" style="font-size:10px;">{{ c.state }}</span>
                <div class="btn-group" style="margin-left:auto;">
                  <!-- Mirror member: Detach + Replace -->
                  <button v-if="v.name.startsWith('mirror')" class="btn-secondary btn-tiny" @click="detachDisk(c.name)">{{ t('zfs.detachDisk') }}</button>
                  <!-- Any disk: Replace -->
                  <button class="btn-secondary btn-tiny" @click="replaceDisk(c.name)">{{ t('zfs.replaceDisk') }}</button>
                </div>
              </div>
            </div>
          </template>
        </div>
      </template>
      <div v-if="!info.vdevs || !info.vdevs.length" class="empty">{{ t('zfs.noVdev') }}</div>
    </div>

    <div v-if="info.error_text && !info.error_text.includes('No known')" class="card" style="border-color:var(--danger);">
      <div class="card-title" style="color:var(--danger);">{{ t('zfs.errors') }}</div>
      <p style="font-size:13px;color:var(--danger);">{{ info.error_text }}</p>
    </div>

    <!-- Maintenance -->
    <div class="card">
      <div class="card-title">{{ t('zfs.maintenance') }}</div>
      <div class="flex" style="gap:12px;flex-wrap:wrap;">
        <button class="btn-secondary" @click="scrub">{{ t('zfs.scrubStart') }}</button>
        <button class="btn-secondary" @click="scrubStop">{{ t('zfs.scrubStop') }}</button>
        <button class="btn-secondary" @click="exportPool">{{ t('zfs.poolExport') }}</button>
        <button class="btn-danger" @click="destroyPool">{{ t('zfs.poolDestroy') }}</button>
      </div>
      <p class="text-dim" style="font-size:12px;margin-top:10px;">{{ t('zfs.scrubHint') }}</p>
    </div>

    <PoolManageModal
      :show="showAddVdev"
      mode="add"
      :pool-name="name"
      @close="showAddVdev = false"
      @success="showAddVdev = false; load()"
    />
  </template>
</template>

<style scoped>
.btn-sm { padding: 4px 12px; font-size: 12px; }
.btn-tiny { padding: 2px 8px; font-size: 11px; }
</style>
