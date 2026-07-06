<script setup>
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();

const disks = ref(null);
const error = ref('');

async function copyUuid(uuid) {
  try {
    await navigator.clipboard.writeText(uuid);
    toast.toast(t('disks.uuidCopied'));
  } catch {
    await alert(t('common.operationFailed'), t('disks.copyFailed'));
  }
}

function usedBytes(d) {
  return d.partitions.reduce((s, p) => s + p.mediasize_bytes, 0);
}

onMounted(async () => {
  try {
    disks.value = await api.get('/api/filesystem/disks');
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('disks.title') }}</h1>
    <p>{{ t('disks.subtitle') }}</p>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!disks" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  <div v-else-if="!disks.length" class="card empty">{{ t('fs.noDisks') }}</div>

  <template v-else>
    <div v-for="d in disks" :key="d.name" class="card" style="padding:0;">
      <div class="flex" style="justify-content:space-between;align-items:center;padding:14px 18px;border-bottom:1px solid var(--border);">
        <div class="flex" style="align-items:center;gap:8px;">
          <span class="mono" style="font-size:18px;font-weight:700;">{{ d.name }}</span>
          <span class="text-dim">·</span>
          <span>{{ d.descr || '—' }}</span>
        </div>
        <div class="flex" style="align-items:center;gap:8px;">
          <span v-if="d.scheme" class="badge badge-dim">{{ d.scheme }}</span>
          <span v-else class="badge badge-dim">{{ t('disks.noPartitionTable') }}</span>
          <span v-if="d.state" :class="['badge', d.state === 'OK' ? 'badge-success' : 'badge-warn']">{{ d.state }}</span>
          <span class="text-dim mono" style="font-size:13px;">{{ fmtBytes(d.size_bytes) }}</span>
        </div>
      </div>

      <div class="stat-grid" style="margin:16px 18px;">
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.devicePath') }}</div><div class="mono" style="font-size:13px;word-break:break-all;">/dev/{{ d.name }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.model') }}</div><div style="font-size:13px;">{{ d.descr || '—' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.totalSize') }}</div><div class="mono" style="font-size:13px;">{{ fmtBytes(d.size_bytes) }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.sectorSize') }}</div><div class="mono" style="font-size:13px;">{{ d.sectorsize ? d.sectorsize + ' B' : '—' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.serialIdent') }}</div><div class="mono" style="font-size:13px;">{{ d.ident || '—' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.lunId') }}</div><div class="mono" style="font-size:13px;">{{ d.lunid || '—' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.rpm') }}</div><div style="font-size:13px;">{{ d.rotation_rate === 'unknown' ? t('fs.ssdUnknown') : d.rotation_rate + ' rpm' }}</div></div>
        <div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.accessMode') }}</div><div class="mono" style="font-size:13px;">{{ d.mode || '—' }}</div></div>
        <template v-if="d.scheme"><div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.partScheme') }}</div><div style="font-size:13px;">{{ d.scheme }}</div></div></template>
        <template v-if="d.entries != null"><div><div class="text-dim" style="font-size:11px;margin-bottom:2px;">{{ t('disks.gptEntries') }}</div><div class="mono" style="font-size:13px;">{{ d.entries }}</div></div></template>
      </div>

      <div style="padding:0 18px 16px;">
        <div class="flex" style="justify-content:space-between;font-size:12px;margin-bottom:6px;">
          <span class="text-dim">{{ t('disks.allocated', { used: fmtBytes(usedBytes(d)), free: fmtBytes(Math.max(0, d.size_bytes - usedBytes(d))) }) }}</span>
          <span class="mono text-dim">{{ (usedBytes(d) / d.size_bytes * 100).toFixed(0) }}%</span>
        </div>
        <div class="bar-wrap">
          <div :class="['bar', usedBytes(d) / d.size_bytes * 100 > 80 ? 'bar-swap' : 'bar-cpu']" :style="{ width: Math.min(100, usedBytes(d) / d.size_bytes * 100) + '%' }"></div>
        </div>
      </div>

      <div style="padding:0 18px 18px;">
        <h2 style="font-size:14px;margin:8px 0 8px;">{{ t('disks.partTable', { n: d.partitions.length }) }}</h2>
        <table>
          <thead><tr>
            <th>{{ t('common.device') }}</th><th>{{ t('common.type') }}</th><th>{{ t('disks.label') }}</th>
            <th>{{ t('common.size') }}</th><th>{{ t('disks.startSector') }}</th><th>{{ t('disks.endSector') }}</th><th>{{ t('disks.uuid') }}</th>
          </tr></thead>
          <tbody>
            <tr v-if="!d.partitions.length"><td colspan="7" class="empty">{{ t('disks.noPartitions') }}</td></tr>
            <tr v-for="p in [...d.partitions].sort((a, b) => a.index - b.index)" :key="p.name">
              <td class="mono"><strong>{{ p.name }}</strong></td>
              <td><span class="badge badge-dim">{{ p.type }}</span></td>
              <td>{{ p.label || '—' }}</td>
              <td class="mono">{{ fmtBytes(p.mediasize_bytes) }}</td>
              <td class="mono text-dim">{{ p.start }}</td>
              <td class="mono text-dim">{{ p.end }}</td>
              <td class="mono text-dim" style="font-size:11px;">
                <span class="uuid-tip" style="cursor:pointer;" @click="copyUuid(p.rawuuid)">{{ p.rawuuid.slice(0, 8) }}…</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </template>
</template>
