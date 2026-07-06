<script setup>
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';

const { t } = useI18n();
const router = useRouter();
const pools = ref(null);
const error = ref('');

function barClass(pct) {
  return pct > 80 ? 'bar-swap' : 'bar-mem';
}

onMounted(async () => {
  try {
    pools.value = await api.get('/api/zfs/pools');
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <h1>Zpool</h1>
    <p>{{ t('zfs.poolsSubtitle') }}</p>
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
        <span class="text-dim" style="font-size:13px;">{{ t('zfs.usedPct', { pct: p.capacity_pct.toFixed(0) }) }}</span>
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
</template>
