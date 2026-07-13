<script setup>
import { ref, computed, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';
import { fmtBytes } from '../../lib/format.js';

const props = defineProps({
  show: Boolean,
  mode: { type: String, default: 'create' },
  poolName: { type: String, default: '' },
});

const emit = defineEmits(['close', 'success']);
const { t, locale } = useI18n();

const disks = ref([]);
const loading = ref(false);
const submitting = ref(false);
const error = ref('');
const poolName = ref('');
const ashift = ref('auto');
const mountpoint = ref('');
const vdevGroups = ref([]);

const VDEV_TYPES = [
  {
    value: 'disk', minDisks: 1,
    descEn: 'No redundancy. Each disk is an independent vdev; pool capacity = sum of all selected disks. Any disk failure destroys the entire pool.',
    descZh: '无冗余。每块盘是独立的 vdev，存储池容量 = 所选磁盘容量之和。任意一块盘故障将导致整个存储池数据丢失。',
  },
  {
    value: 'mirror', minDisks: 2,
    descEn: 'Full redundancy (mirror). Data is identical on all disks. Capacity = smallest disk. Tolerates N-1 disk failures. Best random read performance.',
    descZh: '完全冗余（镜像）。数据在所有磁盘上完全相同，容量 = 最小磁盘容量。可容忍 N-1 块盘故障。随机读取性能最佳。',
  },
  {
    value: 'raidz1', minDisks: 3,
    descEn: 'Single parity (RAID-Z1). Capacity = (N-1) × smallest disk. Tolerates 1 disk failure. Good space efficiency for 3-5 disks.',
    descZh: '单校验（RAID-Z1）。容量 = (N-1) × 最小磁盘容量。可容忍 1 块盘故障。适合 3-5 块盘，空间利用率高。',
  },
  {
    value: 'raidz2', minDisks: 4,
    descEn: 'Double parity (RAID-Z2). Capacity = (N-2) × smallest disk. Tolerates 2 simultaneous disk failures. Recommended for most arrays.',
    descZh: '双校验（RAID-Z2）。容量 = (N-2) × 最小磁盘容量。可容忍同时 2 块盘故障。适合大多数阵列配置，推荐使用。',
  },
  {
    value: 'raidz3', minDisks: 5,
    descEn: 'Triple parity (RAID-Z3). Capacity = (N-3) × smallest disk. Tolerates 3 simultaneous disk failures. For large arrays or critical data.',
    descZh: '三校验（RAID-Z3）。容量 = (N-3) × 最小磁盘容量。可容忍同时 3 块盘故障。适用于大型阵列或关键数据。',
  },
];

function vdevLabel(type) {
  switch (type) {
    case 'disk': return t('zfs.vdevStripe');
    case 'mirror': return t('zfs.vdevMirror');
    case 'raidz1': return 'RAID-Z1';
    case 'raidz2': return 'RAID-Z2';
    case 'raidz3': return 'RAID-Z3';
    default: return type;
  }
}

function vdevDesc(type) {
  const v = VDEV_TYPES.find(v => v.value === type);
  if (!v) return '';
  return locale.value === 'zh' ? v.descZh : v.descEn;
}

function minDisksFor(type) {
  return VDEV_TYPES.find(v => v.value === type)?.minDisks || 1;
}

watch(() => props.show, async (val) => {
  if (val) {
    poolName.value = '';
    ashift.value = 'auto';
    mountpoint.value = '';
    vdevGroups.value = [{ vdevType: 'disk', selectedDisks: [] }];
    error.value = '';
    await fetchDisks();
  }
});

async function fetchDisks() {
  loading.value = true;
  try {
    disks.value = await api.get('/api/zfs/pools/available-disks');
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

function addGroup() {
  vdevGroups.value.push({ vdevType: 'disk', selectedDisks: [] });
}

function removeGroup(idx) {
  vdevGroups.value.splice(idx, 1);
}

function isDiskSelected(diskName, excludeIdx) {
  return vdevGroups.value.some((g, i) => i !== excludeIdx && g.selectedDisks.includes(diskName));
}

function selectableDisks(groupIdx) {
  return disks.value.filter(d => !d.in_use && !isDiskSelected(d.name, groupIdx));
}

function toggleDisk(group, diskName) {
  const idx = group.selectedDisks.indexOf(diskName);
  if (idx >= 0) {
    group.selectedDisks.splice(idx, 1);
  } else {
    group.selectedDisks.push(diskName);
  }
}

const canSubmit = computed(() => {
  if (props.mode === 'create' && !poolName.value.trim()) return false;
  if (vdevGroups.value.length === 0) return false;
  return vdevGroups.value.every(g => g.selectedDisks.length >= minDisksFor(g.vdevType));
});

async function submit() {
  if (submitting.value) return;
  error.value = '';
  submitting.value = true;
  const vdevs = vdevGroups.value.map(g => ({
    vdev_type: g.vdevType,
    disks: g.selectedDisks,
  }));
  try {
    if (props.mode === 'create') {
      const body = { name: poolName.value.trim(), vdevs };
      if (ashift.value !== 'auto') body.ashift = parseInt(ashift.value);
      if (mountpoint.value.trim()) body.mountpoint = mountpoint.value.trim();
      await api.post('/api/zfs/pools', body);
    } else {
      await api.post(`/api/zfs/pools/${props.poolName}/add`, { vdevs });
    }
    emit('success');
  } catch (e) {
    error.value = e.message || t('common.operationFailed');
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div v-if="show" class="modal-overlay">
    <div class="modal pool-modal">
      <h3>{{ mode === 'create' ? t('zfs.poolCreateTitle') : t('zfs.addVdevTitle', { name: poolName }) }}</h3>

      <div v-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
      <template v-else>
        <div v-if="error" class="empty" style="color:var(--danger);margin-bottom:12px;">{{ error }}</div>

        <!-- Pool name (create mode only) -->
        <div v-if="mode === 'create'" class="field">
          <label>{{ t('zfs.poolNameLabel') }} <span style="color:var(--danger)">*</span></label>
          <input v-model="poolName" :placeholder="t('zfs.poolNamePlaceholder')" />
        </div>

        <!-- ashift (create mode only) -->
        <div v-if="mode === 'create'" class="field">
          <label>{{ t('zfs.ashiftLabel') }}</label>
          <select v-model="ashift">
            <option value="auto">{{ t('zfs.ashiftAuto') }}</option>
            <option value="9">512B (9)</option>
            <option value="12">4K (12)</option>
            <option value="13">8K (13)</option>
            <option value="14">16K (14)</option>
          </select>
        </div>

        <!-- Mountpoint (create mode only) -->
        <div v-if="mode === 'create'" class="field">
          <label>{{ t('zfs.mountpointLabel') }}</label>
          <input v-model="mountpoint" :placeholder="t('zfs.mountpointPlaceholder')" />
        </div>

        <!-- VDEV groups -->
        <div class="field">
          <label>{{ t('zfs.vdevGroups') }}</label>
        </div>

        <div v-for="(g, gi) in vdevGroups" :key="gi" class="vdev-builder-group">
          <div class="flex" style="justify-content:space-between;align-items:center;margin-bottom:8px;">
            <select v-model="g.vdevType" style="flex:1;">
              <option v-for="v in VDEV_TYPES" :key="v.value" :value="v.value">
                {{ vdevLabel(v.value) }} ({{ t('zfs.minDisksHint', { n: v.minDisks }) }})
              </option>
            </select>
            <button v-if="vdevGroups.length > 1" class="btn-secondary btn-sm" style="margin-left:8px;" @click="removeGroup(gi)">
              {{ t('common.remove') }}
            </button>
          </div>
          <p class="vdev-desc">{{ vdevDesc(g.vdevType) }}</p>

          <div class="disk-list">
            <label
              v-for="d in selectableDisks(gi)"
              :key="d.name"
              class="disk-list-row"
              :class="{ selected: g.selectedDisks.includes(d.name) }"
            >
              <input
                type="checkbox"
                :checked="g.selectedDisks.includes(d.name)"
                @change="toggleDisk(g, d.name)"
              />
              <span class="mono disk-name">{{ d.name }}</span>
              <span class="disk-descr">{{ d.descr || '—' }}</span>
              <span class="disk-size">{{ fmtBytes(d.size_bytes) }}</span>
            </label>
          </div>
          <div v-if="selectableDisks(gi).length === 0" class="empty" style="font-size:12px;padding:8px 0;">
            {{ disks.length === 0 ? t('zfs.noAvailableDisks') : t('common.pleaseSelect') }}
          </div>
        </div>

        <button class="btn-secondary btn-sm" style="margin-top:8px;" @click="addGroup">
          + {{ t('zfs.addVdevGroup') }}
        </button>

        <div class="modal-actions">
          <button class="btn-secondary" :disabled="submitting" @click="emit('close')">{{ t('common.cancel') }}</button>
          <button :disabled="!canSubmit || submitting" @click="submit">
            <span v-if="submitting" class="spinner" style="width:14px;height:14px;"></span>
            {{ mode === 'create' ? t('common.create') : t('common.add') }}
          </button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.pool-modal { max-width: 640px; max-height: 85vh; overflow-y: auto; }
.vdev-builder-group {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px;
  margin-bottom: 12px;
  background: var(--bg-elev2);
}
.btn-sm { padding: 4px 10px; font-size: 12px; }
.vdev-desc {
  font-size: 12px; color: var(--text-dim); line-height: 1.5;
  margin: 0 0 10px; padding: 6px 10px;
  background: var(--bg-elev); border-radius: var(--radius);
}
.disk-list { display: flex; flex-direction: column; gap: 2px; }
.disk-list-row {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 10px;
  border-radius: var(--radius);
  cursor: pointer;
  font-size: 13px;
  transition: background 0.12s;
}
.disk-list-row:hover { background: var(--bg-elev); }
.disk-list-row.selected { background: rgba(59,130,246,0.12); }
.disk-list-row input[type="checkbox"] { width: 14px; height: 14px; flex-shrink: 0; }
.disk-name { font-weight: 600; min-width: 80px; }
.disk-descr { color: var(--text-dim); flex: 1; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.disk-size { color: var(--text-dim); font-size: 12px; white-space: nowrap; }
</style>
