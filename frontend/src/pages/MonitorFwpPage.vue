<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAlert } from '../composables/useDialog.js';
import { api } from '../lib/api.js';
import { fmtBytes, fmtUptime } from '../lib/format.js';

const { t } = useI18n();
const alert = useAlert();

const stats = ref(null);
const tokio = ref(null);
const tasks = ref(null);
const loading = ref(true);
const lastUpdate = ref(null);
const autoRefresh = ref(false);
let timer = null;

async function load() {
  try {
    const [memRes, tokioRes, tasksRes] = await Promise.all([
      api.get('/api/debug/jemalloc-stats'),
      api.get('/api/debug/tokio-metrics'),
      api.get('/api/scheduler/status'),
    ]);
    stats.value = memRes;
    tokio.value = tokioRes;
    tasks.value = tasksRes;
    lastUpdate.value = new Date();
    loading.value = false;
  } catch (e) {
    loading.value = false;
    alert(t('debug.loadFailed'), e.message || String(e));
  }
}

function setAutoRefresh(on) {
  autoRefresh.value = on;
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
  if (on) {
    timer = setInterval(load, 3000);
  }
}

onMounted(load);
onUnmounted(() => {
  if (timer) clearInterval(timer);
});

// jemalloc 囤积未还 = resident - allocated；可能为 null 或负（metadata 计入驻留时）。
const hoarded = computed(() => {
  if (!stats.value) return null;
  const r = stats.value.resident;
  const a = stats.value.allocated;
  if (r == null || a == null) return null;
  return Math.max(0, r - a);
});

const residentRatio = computed(() => {
  if (!stats.value) return null;
  const r = stats.value.resident;
  const a = stats.value.allocated;
  if (!r || !a) return null;
  return (r / a).toFixed(2);
});

function fmtDuration(ms) {
  if (ms == null) return '—';
  if (ms < 1000) return ms + ' ms';
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + ' s';
  const m = Math.floor(s / 60);
  const rs = s % 60;
  if (m < 60) return m + ' m ' + rs + ' s';
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return h + ' h ' + rm + ' m';
}

// 四类内存分类（基于进程 RSS）：
//   在用       = allocated               程序真正使用的（jemalloc 内）
//   其它       = process_rss - resident   非 jemalloc：代码段、调试符号、栈、共享库
//   待回收     = resident - active        jemalloc 脏/模糊页，OS 可回收
//   不可回收   = active - allocated       jemalloc 内部碎片
function breakdown() {
  const s = stats.value;
  if (!s) return null;
  const a = s.allocated ?? 0;
  const ac = s.active ?? a;
  const r = s.resident ?? 0;
  const prss = s.process_rss ?? 0;
  const used = a;
  const other = Math.max(0, prss - r);
  const cached = Math.max(0, r - ac);
  const overhead = Math.max(0, ac - a);
  const total = used + other + cached + overhead || 1;
  const pct = (v) => ((v / total) * 100).toFixed(1);
  return { used, other, cached, overhead, pct };
}
const memBreakdown = computed(breakdown);

function fmtTs(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleString();
}

// fwp 进程启动至今的秒数，交给共享 fmtUptime() 格式化（与仪表盘一致）。
const fwpUptimeSec = computed(() => {
  const sa = tasks.value?.started_at;
  if (!sa) return null;
  return Math.max(0, Math.floor(Date.now() / 1000) - sa);
});
</script>

<template>
  <div class="page-header">
    <div>
      <h1>{{ t('nav.monitorFwp') }}</h1>
      <p>{{ t('debug.subtitle') }}</p>
    </div>
    <div style="margin-left:auto; display:flex; gap:8px; align-items:center;">
      <span v-if="lastUpdate" class="text-dim" style="font-size:12px;">
        {{ t('debug.updatedAt') }} {{ lastUpdate.toLocaleTimeString() }}
      </span>
      <button class="btn-secondary btn-sm" @click="load" :disabled="loading">
        <i class="fa-solid fa-rotate-right"></i> {{ t('common.refresh') }}
      </button>
      <div class="filter-group">
        <button :class="['filter-btn', { active: !autoRefresh }]" @click="setAutoRefresh(false)">
          {{ t('debug.refreshManual') }}
        </button>
        <button :class="['filter-btn', { active: autoRefresh }]" @click="setAutoRefresh(true)">
          {{ t('debug.refreshAuto') }}
        </button>
      </div>
    </div>
  </div>

  <div v-if="loading && !stats" class="text-dim" style="text-align:center;padding:40px;">
    {{ t('common.loading') }}
  </div>

  <template v-else-if="stats && memBreakdown">
    <!-- ── 进程总览：RSS + 启动时长 ──────────────────── -->
    <div class="debug-heroes">
      <div class="card debug-rss-total">
        <div class="debug-rss-label">{{ t('debug.totalRss') }}</div>
        <div class="debug-rss-value">{{ stats.process_rss == null ? '—' : fmtBytes(stats.process_rss) }}</div>
        <div class="debug-rss-hint">{{ t('debug.totalRssHint') }}</div>
      </div>
      <div class="card debug-rss-total">
        <div class="debug-rss-label">{{ t('dash.uptime') }}</div>
        <div class="debug-rss-value">{{ fwpUptimeSec == null ? '—' : fmtUptime(fwpUptimeSec) }}</div>
        <div class="debug-rss-hint">{{ t('debug.uptimeHint') }}</div>
      </div>
    </div>

    <!-- ── 内存分布堆叠条 ─────────────────────────────── -->
    <div class="debug-bar-wrap">
      <div class="debug-bar">
        <div class="debug-bar-seg seg-used"     :style="{ width: memBreakdown.pct(memBreakdown.used) + '%' }"     :title="t('debug.catUsed') + ': ' + fmtBytes(memBreakdown.used)"></div>
        <div class="debug-bar-seg seg-other"    :style="{ width: memBreakdown.pct(memBreakdown.other) + '%' }"    :title="t('debug.catOther') + ': ' + fmtBytes(memBreakdown.other)"></div>
        <div class="debug-bar-seg seg-cached"   :style="{ width: memBreakdown.pct(memBreakdown.cached) + '%' }"   :title="t('debug.catCached') + ': ' + fmtBytes(memBreakdown.cached)"></div>
        <div class="debug-bar-seg seg-overhead" :style="{ width: memBreakdown.pct(memBreakdown.overhead) + '%' }" :title="t('debug.catOverhead') + ': ' + fmtBytes(memBreakdown.overhead)"></div>
      </div>
      <div class="debug-bar-legend">
        <span><i class="dot dot-used"></i>{{ t('debug.catUsed') }}</span>
        <span><i class="dot dot-other"></i>{{ t('debug.catOther') }}</span>
        <span><i class="dot dot-cached"></i>{{ t('debug.catCached') }}</span>
        <span><i class="dot dot-overhead"></i>{{ t('debug.catOverhead') }}</span>
      </div>
    </div>

    <!-- ── 内存去向：通俗分类 ─────────────────────────── -->
    <div class="debug-breakdown">
      <div class="debug-cat debug-cat-used">
        <div class="debug-cat-label"><i class="fa-solid fa-microchip"></i> {{ t('debug.catUsed') }}</div>
        <div class="debug-cat-value">{{ fmtBytes(memBreakdown.used) }}</div>
        <div class="debug-cat-pct">{{ memBreakdown.pct(memBreakdown.used) }}%</div>
        <div class="debug-cat-desc">{{ t('debug.catUsedDesc') }}</div>
      </div>
      <div class="debug-cat debug-cat-other">
        <div class="debug-cat-label"><i class="fa-solid fa-layer-group"></i> {{ t('debug.catOther') }}</div>
        <div class="debug-cat-value">{{ fmtBytes(memBreakdown.other) }}</div>
        <div class="debug-cat-pct">{{ memBreakdown.pct(memBreakdown.other) }}%</div>
        <div class="debug-cat-desc">{{ t('debug.catOtherDesc') }}</div>
      </div>
      <div class="debug-cat debug-cat-cached">
        <div class="debug-cat-label"><i class="fa-solid fa-clock-rotate-left"></i> {{ t('debug.catCached') }}</div>
        <div class="debug-cat-value">{{ fmtBytes(memBreakdown.cached) }}</div>
        <div class="debug-cat-pct">{{ memBreakdown.pct(memBreakdown.cached) }}%</div>
        <div class="debug-cat-desc">{{ t('debug.catCachedDesc') }}</div>
      </div>
      <div class="debug-cat debug-cat-overhead">
        <div class="debug-cat-label"><i class="fa-solid fa-gears"></i> {{ t('debug.catOverhead') }}</div>
        <div class="debug-cat-value">{{ fmtBytes(memBreakdown.overhead) }}</div>
        <div class="debug-cat-pct">{{ memBreakdown.pct(memBreakdown.overhead) }}%</div>
        <div class="debug-cat-desc">{{ t('debug.catOverheadDesc') }}</div>
      </div>
    </div>

    <!-- ── 技术细节 ───────────────────────────────────── -->
    <div class="card debug-details">
      <div class="debug-details-title">
        <i class="fa-solid fa-flask"></i> {{ t('debug.detailsTitle') }}
      </div>
      <div class="debug-details-body">
        <p class="text-dim" style="margin:0 0 10px 0;">{{ t('debug.detailsIntro') }}</p>
        <div class="debug-grid">
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.allocated') }}</div>
            <div class="debug-value">{{ stats.allocated == null ? '—' : fmtBytes(stats.allocated) }}</div>
            <div class="debug-hint">{{ t('debug.allocatedHint') }}</div>
          </div>
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.active') }}</div>
            <div class="debug-value">{{ stats.active == null ? '—' : fmtBytes(stats.active) }}</div>
            <div class="debug-hint">{{ t('debug.activeHint') }}</div>
          </div>
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.metadata') }}</div>
            <div class="debug-value">{{ stats.metadata == null ? '—' : fmtBytes(stats.metadata) }}</div>
            <div class="debug-hint">{{ t('debug.metadataHint') }}</div>
          </div>
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.resident') }}</div>
            <div class="debug-value">{{ stats.resident == null ? '—' : fmtBytes(stats.resident) }}</div>
            <div class="debug-hint">{{ t('debug.residentHint') }}</div>
          </div>
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.mapped') }}</div>
            <div class="debug-value">{{ stats.mapped == null ? '—' : fmtBytes(stats.mapped) }}</div>
            <div class="debug-hint">{{ t('debug.mappedHint') }}</div>
          </div>
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.retained') }}</div>
            <div class="debug-value">{{ stats.retained == null ? '—' : fmtBytes(stats.retained) }}</div>
            <div class="debug-hint">{{ t('debug.retainedHint') }}</div>
          </div>
          <div class="debug-stat">
            <div class="debug-label">{{ t('debug.totalRss') }}</div>
            <div class="debug-value">{{ stats.process_rss == null ? '—' : fmtBytes(stats.process_rss) }}</div>
            <div class="debug-hint">{{ t('debug.totalRssHint') }}</div>
          </div>
        </div>
        <div class="debug-summary">
          <div class="debug-summary-row">
            <span>{{ t('debug.hoarded') }}</span>
            <strong>{{ fmtBytes(hoarded) }}</strong>
            <span class="text-dim" style="margin-left:8px;">
              (resident − allocated；jemalloc {{ t('debug.hoardHint') }})
            </span>
          </div>
          <div class="debug-summary-row">
            <span>{{ t('debug.residentRatio') }}</span>
            <strong>{{ residentRatio }}×</strong>
            <span class="text-dim" style="margin-left:8px;">
              (resident ÷ allocated；{{ t('debug.ratioHint') }})
            </span>
          </div>
        </div>

        <!-- Tokio runtime -->
        <div v-if="tokio" class="debug-tokio-section">
          <div class="debug-tokio-title">
            <i class="fa-solid fa-bolt"></i> {{ t('debug.tokioTitle') }}
          </div>
          <div class="debug-grid">
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioWorkers') }}</div>
              <div class="debug-value">{{ tokio.workers_count }}</div>
              <div class="debug-hint">{{ t('debug.tokioWorkersHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioLiveTasks') }}</div>
              <div class="debug-value">{{ tokio.live_tasks_count }}</div>
              <div class="debug-hint">{{ t('debug.tokioLiveTasksHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioTotalPolls') }}</div>
              <div class="debug-value">{{ tokio.total_polls_count.toLocaleString() }}</div>
              <div class="debug-hint">{{ t('debug.tokioTotalPollsHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioBusyDuration') }}</div>
              <div class="debug-value">{{ fmtDuration(tokio.total_busy_duration_ms) }}</div>
              <div class="debug-hint">{{ t('debug.tokioBusyDurationHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioGlobalQueue') }}</div>
              <div class="debug-value">{{ tokio.global_queue_depth }}</div>
              <div class="debug-hint">{{ t('debug.tokioGlobalQueueHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioLocalQueue') }}</div>
              <div class="debug-value">{{ tokio.total_local_queue_depth }}</div>
              <div class="debug-hint">{{ t('debug.tokioLocalQueueHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioStealCount') }}</div>
              <div class="debug-value">{{ tokio.total_steal_count.toLocaleString() }}</div>
              <div class="debug-hint">{{ t('debug.tokioStealCountHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioBlockingQueue') }}</div>
              <div class="debug-value">{{ tokio.blocking_queue_depth }}</div>
              <div class="debug-hint">{{ t('debug.tokioBlockingQueueHint') }}</div>
            </div>
            <div class="debug-stat">
              <div class="debug-label">{{ t('debug.tokioBlockingThreads') }}</div>
              <div class="debug-value">{{ tokio.blocking_threads_count }}</div>
              <div class="debug-hint">{{ t('debug.tokioBlockingThreadsHint') }}</div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- ── 定时任务 ─────────────────────────────────────── -->
    <div v-if="tasks" class="card debug-tasks-section" style="padding:0;">
      <div class="debug-details-title">
        <i class="fa-solid fa-clock-rotate-left"></i> {{ t('tasks.subtitle') }}
        <span v-if="tasks.started_at" class="text-dim" style="margin-left:auto; font-size:12px; font-weight:normal;">
          {{ t('dash.uptime') }}: {{ fmtUptime(fwpUptimeSec) }}
        </span>
      </div>
      <table>
        <thead><tr>
          <th>{{ t('common.name') }}</th>
          <th>{{ t('cron.schedule') }}</th>
          <th>{{ t('tasks.runCount') }}</th>
          <th>{{ t('tasks.lastRun') }}</th>
          <th>{{ t('tasks.nextRun') }}</th>
          <th>{{ t('common.status') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="!tasks.jobs.length"><td colspan="6" class="empty">{{ t('common.noData') }}</td></tr>
          <tr v-for="job in tasks.jobs" :key="job.name">
            <td class="mono">{{ job.name }}</td>
            <td class="mono">{{ job.schedule }}</td>
            <td>{{ job.run_count.toLocaleString() }}</td>
            <td class="text-dim">{{ fmtTs(job.last_run_ts) }}</td>
            <td class="text-dim">{{ fmtTs(job.next_run_ts) }}</td>
            <td>
              <span v-if="job.last_error" class="tag tag-error">{{ t('common.error') }}</span>
              <span v-else-if="job.run_count > 0" class="tag tag-ok">{{ t('common.ok') }}</span>
              <span v-else class="tag tag-dim">{{ t('tasks.pending') }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>

<style scoped>
/* ── RSS total header ──────────────────────────────────── */
.debug-rss-total {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 12px 16px;
  margin-bottom: 12px;
  border-left: 4px solid var(--accent);
}
.debug-heroes {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-bottom: 12px;
}
.debug-heroes .debug-rss-total {
  margin-bottom: 0;
  flex: 1;
  min-width: 260px;
}
.debug-rss-label {
  font-size: 13px;
  color: var(--text-dim);
  white-space: nowrap;
}
.debug-rss-value {
  font-size: 28px;
  font-weight: 700;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
.debug-rss-hint {
  font-size: 12px;
  color: var(--text-dim);
  margin-left: auto;
}

/* ── Breakdown: 5 category cards ─────────────────────── */
.debug-breakdown {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px;
  margin-bottom: 12px;
}
.debug-cat {
  background: var(--bg-elev);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 14px 16px;
  border-left: 4px solid var(--text-dim);
}
.debug-cat-used     { border-left-color: var(--success); }
.debug-cat-other    { border-left-color: #a78bfa; }
.debug-cat-cached   { border-left-color: #f59e0b; }
.debug-cat-overhead { border-left-color: var(--text-dim); }
.debug-cat-label {
  font-size: 13px;
  color: var(--text-dim);
  margin-bottom: 6px;
}
.debug-cat-label i {
  width: 14px;
  margin-right: 4px;
}
.debug-cat-value {
  font-size: 24px;
  font-weight: 600;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
.debug-cat-pct {
  font-size: 13px;
  color: var(--text-dim);
  margin-top: 2px;
  font-variant-numeric: tabular-nums;
}
.debug-cat-desc {
  font-size: 12px;
  color: var(--text-dim);
  margin-top: 10px;
  line-height: 1.5;
}

/* ── Stacked bar + legend ─────────────────────────────── */
.debug-bar-wrap {
  margin-bottom: 12px;
}
.debug-bar {
  display: flex;
  height: 14px;
  border-radius: 7px;
  overflow: hidden;
  background: var(--bg-elev);
  border: 1px solid var(--border);
}
.debug-bar-seg {
  height: 100%;
  min-width: 0;
}
.seg-used     { background: var(--success); }
.seg-other    { background: #a78bfa; }
.seg-cached   { background: #f59e0b; }
.seg-overhead { background: var(--text-dim); }
.debug-bar-legend {
  display: flex;
  gap: 16px;
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-dim);
  flex-wrap: wrap;
}
.debug-bar-legend .dot {
  display: inline-block;
  width: 10px;
  height: 10px;
  border-radius: 2px;
  margin-right: 4px;
  vertical-align: middle;
  font-style: normal;
}
.dot-used     { background: var(--success); }
.dot-other    { background: #a78bfa; }
.dot-cached   { background: #f59e0b; }
.dot-overhead { background: var(--text-dim); }

/* ── Technical details section ───────────────────────── */
.debug-details {
  padding: 0;
}
.debug-details-title {
  padding: 12px 16px;
  color: var(--text);
  font-weight: 500;
  border-bottom: 1px solid var(--border);
}
.debug-details-title i {
  width: 14px;
  margin-right: 6px;
  color: var(--text-dim);
}
.debug-details-body {
  padding: 14px 16px;
}

/* ── Technical grid ───────────────────────────────────── */
.debug-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 10px;
  margin-bottom: 12px;
}
.debug-stat {
  background: var(--bg-elev2);
  border-radius: 6px;
  padding: 10px 12px;
}
.debug-label {
  font-size: 11px;
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-bottom: 4px;
}
.debug-value {
  font-size: 18px;
  font-weight: 600;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
.debug-hint {
  font-size: 11px;
  color: var(--text-dim);
  margin-top: 6px;
  line-height: 1.4;
}

/* ── Summary rows ─────────────────────────────────────── */
.debug-summary {
  border-top: 1px dashed var(--border);
  padding-top: 10px;
}
.debug-summary-row {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 6px 0;
  border-bottom: 1px dashed var(--border);
}
.debug-summary-row:last-child {
  border-bottom: none;
}
.debug-summary-row strong {
  font-variant-numeric: tabular-nums;
  color: var(--accent);
  min-width: 90px;
  display: inline-block;
}

/* ── Tokio runtime section ─────────────────────────────── */
.debug-tokio-section {
  margin-top: 16px;
  border-top: 1px dashed var(--border);
  padding-top: 14px;
}
.debug-tokio-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-dim);
  margin-bottom: 10px;
}
.debug-tokio-title i {
  width: 14px;
  margin-right: 4px;
  color: #60a5fa;
}

/* ── Tasks table section ──────────────────────────────── */
.debug-tasks-section {
  margin-top: 12px;
}
.debug-tasks-section .debug-details-title {
  display: flex;
  align-items: center;
}
</style>
