<script setup>
import { ref, onMounted } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const toast = useToast();
const alert = useAlert();

const name = route.params.name;
const info = ref(null);
const error = ref('');

function healthBadge(health) {
  if (health === 'ONLINE') return 'badge-success';
  if (health === 'DEGRADED') return 'badge-warn';
  return 'badge-danger';
}

function barClass(pct) {
  return pct > 80 ? 'bar-swap' : 'bar-mem';
}

async function load() {
  try {
    info.value = await api.get(`/api/zfs/pools/${name}`);
  } catch (err) {
    error.value = err.message || '';
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

onMounted(load);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/zfs/pools" />
      <h1>{{ t('zfs.poolTitle', { name }) }}</h1>
    </div>
    <p>{{ t('zfs.poolDetailSubtitle') }}</p>
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
      <div class="bar-wrap" style="height:16px;">
        <div :class="['bar', barClass(info.capacity_pct || 0)]" :style="{ width: (info.capacity_pct || 0) + '%' }"></div>
      </div>
      <div class="text-dim" style="font-size:12px;margin-top:6px;">{{ fmtBytes(info.allocated) }} / {{ fmtBytes(info.size) }} ({{ (info.capacity_pct || 0).toFixed(1) }}%)</div>
    </div>

    <div v-if="info.scan" class="card">
      <div class="card-title">{{ t('zfs.scrubStatus') }}</div>
      <pre style="font-size:13px;margin:0;white-space:pre-wrap;font-family:var(--font-mono,monospace);line-height:1.6;">{{ info.scan }}</pre>
    </div>

    <div class="card">
      <div class="card-title">{{ t('zfs.vdevTree') }}</div>
      <template v-for="(v, vi) in (info.vdevs || [])" :key="vi">
        <div class="vdev-node">
          <div class="vdev-node-row flex">
            <span class="mono" style="font-size:13px;font-weight:600;">{{ v.name }}</span>
            <span class="badge badge-dim" style="margin:0 8px;font-size:10px;">{{ v.name.startsWith('mirror') ? t('zfs.vdevMirror') : v.name.startsWith('raidz') ? t('zfs.vdevRaidz') : !v.children.length ? t('zfs.vdevDisk') : t('zfs.vdevGeneric') }}</span>
            <span :class="['badge', v.state === 'ONLINE' ? 'badge-success' : v.state === 'DEGRADED' ? 'badge-warn' : 'badge-danger']" style="font-size:10px;">{{ v.state }}</span>
            <span v-if="v.read_errors > 0 || v.write_errors > 0 || v.checksum_errors > 0" class="badge badge-danger" style="font-size:10px;">R:{{ v.read_errors }} W:{{ v.write_errors }} C:{{ v.checksum_errors }}</span>
          </div>
          <template v-for="(c, ci) in v.children" :key="ci">
            <div class="vdev-node" style="margin-left:24px;">
              <div class="vdev-node-row flex">
                <span class="mono" style="font-size:13px;">{{ c.name }}</span>
                <span class="badge badge-dim" style="margin:0 8px;font-size:10px;">{{ !c.children.length ? t('zfs.vdevDisk') : t('zfs.vdevGeneric') }}</span>
                <span :class="['badge', c.state === 'ONLINE' ? 'badge-success' : c.state === 'DEGRADED' ? 'badge-warn' : 'badge-danger']" style="font-size:10px;">{{ c.state }}</span>
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

    <div class="card">
      <div class="card-title">{{ t('zfs.maintenance') }}</div>
      <div class="flex" style="gap:12px;">
        <button class="btn-secondary" @click="scrub">{{ t('zfs.scrubStart') }}</button>
        <button class="btn-secondary" @click="scrubStop">{{ t('zfs.scrubStop') }}</button>
      </div>
      <p class="text-dim" style="font-size:12px;margin-top:10px;">{{ t('zfs.scrubHint') }}</p>
    </div>
  </template>
</template>
