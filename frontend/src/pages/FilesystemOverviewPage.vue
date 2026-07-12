<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';

const { t } = useI18n();
const router = useRouter();

function goToPool(name) {
  router.push({ name: 'zfs-pool-detail', params: { name } });
}
const data = ref(null);
const error = ref('');

function barClass(pct) {
  return pct > 80 ? 'bar-swap' : 'bar-mem';
}

onMounted(async () => {
  try {
    data.value = await api.get('/api/filesystem/overview');
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('fs.title') }}</h1>
    <p>{{ t('fs.subtitle') }}</p>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!data" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <!-- ZFS Pools -->
    <template v-if="data.zpools.length">
    <div class="section-title">{{ t('fs.zfsPools') }}</div>
    <div v-for="p in data.zpools" :key="p.name" class="card clickable" @click="goToPool(p.name)">
      <div class="card-title">{{ t('fs.poolName', { name: p.name }) }}</div>
      <div class="stat-row">
        <span>{{ t('fs.state') }}: <span :class="['badge', p.health === 'ONLINE' ? 'badge-success' : 'badge-danger']">{{ p.health }}</span></span>
        <span>{{ t('common.capacity') }}: <strong>{{ fmtBytes(p.size) }}</strong></span>
        <span>{{ t('common.used') }}: {{ fmtBytes(p.allocated) }} ({{ p.capacity_pct.toFixed(0) }}%)</span>
        <span>{{ t('common.free') }}: {{ fmtBytes(p.free) }}</span>
        <span>{{ t('common.frag') }}: {{ p.fragmentation_pct.toFixed(0) }}%</span>
        <span>{{ t('common.dedup') }}: {{ p.dedup.toFixed(2) }}x</span>
      </div>
      <div class="bar-wrap" style="margin-top:10px;">
        <div :class="['bar', barClass(p.capacity_pct)]" :style="{ width: p.capacity_pct + '%' }"></div>
      </div>
    </div>
    </template>

    <!-- Physical Disks -->
    <div class="section-title" style="margin-top:32px;">{{ t('fs.physicalDisks', { n: data.disks.length }) }}</div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>{{ t('common.device') }}</th><th>{{ t('fs.model') }}</th><th>{{ t('common.size') }}</th><th>{{ t('fs.rpm') }}</th></tr></thead>
        <tbody>
          <tr v-if="!data.disks.length"><td colspan="4" class="empty">{{ t('fs.noDisks') }}</td></tr>
          <tr v-for="d in data.disks" :key="d.name">
            <td class="mono"><strong>{{ d.name }}</strong></td>
            <td>{{ d.descr }}</td>
            <td class="mono">{{ fmtBytes(d.size_bytes) }}</td>
            <td>{{ d.rotation_rate === 'unknown' ? t('fs.ssdUnknown') : d.rotation_rate + ' rpm' }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Mount Points -->
    <div class="section-title" style="margin-top:32px;">{{ t('fs.mountpoints', { n: data.mounts.length }) }}</div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>{{ t('fs.filesystem') }}</th><th>{{ t('fs.mountpoint') }}</th><th>{{ t('fs.fstype') }}</th>
          <th>{{ t('common.total') }}</th><th>{{ t('common.used') }}</th><th>{{ t('common.available') }}</th><th>{{ t('common.usage') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="m in data.mounts" :key="m.mountpoint">
            <td class="mono">{{ m.device }}</td>
            <td class="mono">{{ m.mountpoint }}</td>
            <td><span class="badge badge-dim">{{ m.fstype }}</span></td>
            <td class="mono">{{ m.size > 0 ? fmtBytes(m.size) : '—' }}</td>
            <td class="mono">{{ m.size > 0 ? fmtBytes(m.used) : '—' }}</td>
            <td class="mono">{{ m.size > 0 ? fmtBytes(m.available) : '—' }}</td>
            <td>
              <div v-if="m.size > 0" class="flex">
                <div class="bar-wrap sm" style="width:80px;">
                  <div :class="['bar', barClass(m.capacity_pct)]" :style="{ width: m.capacity_pct + '%' }"></div>
                </div>
                <span class="text-dim mono" style="font-size:11px;">{{ m.capacity_pct.toFixed(0) }}%</span>
              </div>
              <span v-else>—</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
