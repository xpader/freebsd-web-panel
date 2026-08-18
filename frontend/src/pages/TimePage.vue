<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import { clock } from 'vue-clock-lonlyape-v3';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

// ── State ──────────────────────────────────────────────────────────
const status = ref(null);
const ntpConf = ref(null);
const zonesData = ref(null);
const loading = ref(true);
const saving = ref(false);
const actionPending = ref(false);
const showServersModal = ref(false);
const showPeersModal = ref(false);

// Editable copy of NTP servers (decoupled from ntpConf until saved)
const editServers = ref([]);

// Live clock state (computed from server epoch, updated every second)
const liveEpoch = ref(0);
let clockTimer = null;
let ntpTimer = null;

// ── Computed ───────────────────────────────────────────────────────
const liveLocalTime = computed(() => {
  if (!status.value) return '';
  return fmtServerLocal(liveEpoch.value, status.value.utc_offset);
});

// Clock timezone as numeric hours for vue-clock-lonlyape-v3 ("+0800" → 8)
const clockTimezone = computed(() => {
  if (!status.value) return 0;
  const off = status.value.utc_offset;
  const sign = off[0] === '-' ? -1 : 1;
  const h = parseInt(off.slice(1, 3), 10) || 0;
  const m = parseInt(off.slice(3, 5), 10) || 0;
  return sign * (h + m / 60);
});

// Clock visual config, pre-sized for the 128px container. adaptive=false so
// the component's auto-scaler never runs — it multiplies EVERY numeric
// option (including the timezone!) by a resize factor and corrupts the time.
// Sizes = component defaults (300px dial) × 0.42.
const clockStyle = {
  border: { isBorder: true, type: 'circle', width: 120, lineWidth: 1.5, color: 'rgba(128,128,128,.4)' },
  background: { color: 'rgba(128,128,128,.05)' },
  dial: { isDial: true, distance: 0, maxLength: 3.5, minLength: 2, maxWidth: 1.5, minWidth: 1, color: '#888' },
  number: { isNumber: true, type: 'arabic', color: '#999', fontSize: '8px', radius: 48 },
  needle: {
    second: { length: 46, color: '#4a9e94', lineWidth: 1.5, longOut: 4 },
    minute: { length: 42, color: '#bbb', lineWidth: 2, longOut: 3 },
    hour: { length: 30, color: '#ccc', lineWidth: 2.5, longOut: 3 },
  },
};


const ntpRunning = computed(() => status.value?.ntp?.running ?? false);
const ntpEnabled = computed(() => status.value?.ntp?.enabled ?? false);

// ── Formatting helpers ─────────────────────────────────────────────
function fmtServerLocal(epoch, offsetStr) {
  const sign = offsetStr[0] === '-' ? -1 : 1;
  const hours = parseInt(offsetStr.slice(1, 3), 10);
  const mins = parseInt(offsetStr.slice(3, 5), 10);
  const offsetMs = sign * (hours * 3600 + mins * 60) * 1000;
  const d = new Date(epoch * 1000 + offsetMs);
  return d.toISOString().replace('T', ' ').slice(0, 19);
}

function tzOffsetStr(tz) {
  try {
    const d = new Date();
    const local = new Date(d.toLocaleString('en-US', { timeZone: tz }));
    const utc = new Date(d.toLocaleString('en-US', { timeZone: 'UTC' }));
    const diffMin = Math.round((local - utc) / 60000);
    const sign = diffMin >= 0 ? '+' : '-';
    const abs = Math.abs(diffMin);
    const h = Math.floor(abs / 60);
    const m = abs % 60;
    return `UTC${sign}${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
  } catch {
    return '';
  }
}


// ── Data loading ───────────────────────────────────────────────────
async function loadStatus() {
  try {
    status.value = await api.get('/api/time/status');
    liveEpoch.value = status.value.epoch;
  } catch (err) {
    if (loading.value) {
      // Show error only on initial load
    }
  }
}

async function loadNtpConf() {
  try {
    ntpConf.value = await api.get('/api/time/ntp/conf');
    editServers.value = (ntpConf.value.servers || []).map(s => ({ ...s }));
  } catch (err) {
    // non-critical
  }
}

async function loadZones() {
  try {
    zonesData.value = await api.get('/api/time/zones');
  } catch (err) {
    // non-critical
  }
}

async function refreshAll() {
  await Promise.all([loadStatus(), loadNtpConf(), loadZones()]);
  loading.value = false;
}

// ── Actions: System Time ───────────────────────────────────────────
async function setDatetime() {
  // Default to current local time
  const now = new Date(liveEpoch.value * 1000 + (() => {
    const s = status.value.utc_offset[0] === '-' ? -1 : 1;
    const h = parseInt(status.value.utc_offset.slice(1, 3), 10);
    const m = parseInt(status.value.utc_offset.slice(3, 5), 10);
    return s * (h * 3600 + m * 60) * 1000;
  })());
  const defaultVal = now.toISOString().slice(0, 19);

  const result = await formModal(t('time.datetimeTitle'), [
    { key: 'datetime', label: t('time.datetimeLabel'), inputType: 'datetime-local', value: defaultVal },
  ], { submitLabel: t('common.confirm') });
  if (!result) return;

  if (!await confirm(t('time.datetimeTitle'), t('common.confirm'))) return;

  actionPending.value = true;
  try {
    await api.put('/api/time/datetime', { datetime: result.datetime });
    toast.toast(t('common.saved'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

async function syncNow() {
  const result = await formModal(t('time.syncTitle'), [
    { key: 'server', label: t('time.syncServer'), value: 'pool.ntp.org' },
  ], { submitLabel: t('common.confirm') });
  if (!result) return;

  // Warn (but don't block) if ntpd is running — it will be forced to recalibrate.
  if (ntpRunning.value) {
    if (!await confirm(t('time.syncTitle'), t('time.syncNtpdWarn'))) return;
  }

  actionPending.value = true;
  try {
    await api.post('/api/time/sync', { server: result.server });
    toast.toast(t('time.syncSuccess'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

// ── Actions: Timezone ──────────────────────────────────────────────
async function changeTimezone() {
  if (!zonesData.value) return;

  const zoneOptions = zonesData.value.regions.flatMap(r => r.zones).map(z => {
    const off = tzOffsetStr(z);
    return { value: z, label: z, meta: off };
  });

  const result = await formModal(t('time.changeTimezone'), [
    { key: 'zone', type: 'list-select', options: zoneOptions, value: status.value.timezone, required: true },
  ], { submitLabel: t('common.confirm') });
  if (!result) return;

  if (!await confirm(t('time.changeTimezone'), result.zone)) return;

  actionPending.value = true;
  try {
    await api.put('/api/time/timezone', { zone: result.zone });
    toast.toast(t('common.saved'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

// ── Actions: RTC mode ──────────────────────────────────────────────
async function toggleRtcMode() {
  const targetLocal = !status.value.rtc_local;
  const msg = targetLocal ? t('time.rtcConfirmLocal') : t('time.rtcConfirmUtc');
  if (!await confirm(t('time.rtcMode'), msg)) return;

  actionPending.value = true;
  try {
    await api.put('/api/time/rtc-mode', { local: targetLocal });
    toast.toast(t('common.saved'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

// Full explanation of what the RTC mode switch does (opened from the ⓘ icon).
function showRtcHelp() {
  alert(t('time.rtcMode'), t('time.rtcHelp'));
}

// ── Actions: NTP service ───────────────────────────────────────────
async function ntpEnable() {
  if (!await confirm(t('time.ntp'), t('time.ntpEnableConfirm'))) return;
  actionPending.value = true;
  try {
    await api.post('/api/time/ntp/enable');
    toast.toast(t('common.saved'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

async function ntpDisable() {
  if (!await confirm(t('time.ntp'), t('time.ntpDisableConfirm'))) return;
  actionPending.value = true;
  try {
    await api.post('/api/time/ntp/disable');
    toast.toast(t('common.saved'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

async function ntpRestart() {
  actionPending.value = true;
  try {
    await api.post('/api/time/ntp/restart');
    toast.toast(t('common.saved'));
    await loadStatus();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    actionPending.value = false;
  }
}

async function toggleSyncOnStart() {
  const newVal = !ntpConf.value.sync_on_start;
  try {
    await api.put('/api/time/ntp/sync-on-start', { enabled: newVal });
    ntpConf.value.sync_on_start = newVal;
    toast.toast(t('common.saved'));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

// ── Actions: NTP server list ───────────────────────────────────────

// NTP Pool domains resolve to multiple addresses → configure as `pool`.
// Everything else (a specific host) → `server`.
function isPoolHost(host) {
  return /^([0-9a-z-]+\.)*pool\.ntp\.org$/i.test(host.trim());
}

// Auto-classify on host input: user types an address, we decide kind.
function onHostInput(s) {
  s.kind = isPoolHost(s.host) ? 'pool' : 'server';
}

function addServer() {
  editServers.value.push({ kind: 'server', host: '', options: 'iburst' });
}

function removeServer(i) {
  editServers.value.splice(i, 1);
}

async function saveServers() {
  // Validate
  for (const s of editServers.value) {
    if (!s.host.trim()) {
      await alert(t('common.operationFailed'), t('time.serverHost') + ': ' + t('common.pleaseSelect'));
      return;
    }
  }
  saving.value = true;
  try {
    const servers = editServers.value
      .filter(s => s.host.trim())
      .map(s => ({ kind: s.kind, host: s.host.trim(), options: s.options.trim() }));
    await api.put('/api/time/ntp/conf', { servers });
    toast.toast(t('common.saved'));
    showServersModal.value = false;

    // Offer to restart ntpd
    if (ntpRunning.value) {
      if (await confirm(t('time.ntp'), t('time.saveServersConfirmRestart'))) {
        await api.post('/api/time/ntp/restart');
        toast.toast(t('common.saved'));
      }
    }
    await loadStatus();
    await loadNtpConf();
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  } finally {
    saving.value = false;
  }
}

// ── Lifecycle ──────────────────────────────────────────────────────
onMounted(async () => {
  await refreshAll();
  // Live clock: increment epoch every second
  clockTimer = setInterval(() => { liveEpoch.value++; }, 1000);
  // NTP status poll: every 15s
  ntpTimer = setInterval(loadStatus, 15000);
});
onUnmounted(() => {
  if (clockTimer) clearInterval(clockTimer);
  if (ntpTimer) clearInterval(ntpTimer);
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('nav.time') }}</h1>
    <p>{{ t('time.subtitle') }}</p>
  </div>

  <div v-if="loading" class="card" style="padding:1rem;"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else-if="status">
    <!-- ── System Time + Timezone ─────────────────────────────── -->
    <div class="card">
      <div style="display:flex; gap:1.5rem; align-items:center;">
        <clock
          :timezone="clockTimezone"
          :adaptive="false"
          :border="clockStyle.border"
          :background="clockStyle.background"
          :dial="clockStyle.dial"
          :number="clockStyle.number"
          :needle="clockStyle.needle"
          :style="{ width: '128px', height: '128px', flexShrink: 0 }"
        />
        <!-- Left: time display -->
        <div style="flex:1; min-width:0;">
          <div class="time-label">{{ t('time.localTime') }}</div>
          <div class="time-value mono">{{ liveLocalTime }}</div>
          <div class="time-sub">{{ status.timezone }} · UTC{{ status.utc_offset }}</div>
          <div class="time-sub" style="margin-top:1rem;">
            <span class="text-dim">{{ t('time.rtcMode') }}:
              <i class="fa-solid fa-circle-info" style="cursor:pointer; margin-left:.15rem;" :title="t('time.rtcModeHint')" @click="showRtcHelp"></i></span>
            <span :class="['badge', 'badge-sm', status.rtc_local ? 'badge-warn' : 'badge-success']" style="margin:0 .35rem;">
              {{ status.rtc_local ? t('time.localTime') : t('time.rtcUtc') }}
            </span>
            <button class="btn-secondary btn-sm" :disabled="actionPending" @click="toggleRtcMode">
              {{ status.rtc_local ? t('time.setRtcUtc') : t('time.setRtcLocal') }}
            </button>
          </div>
        </div>
        <!-- Right: vertical action buttons -->
        <div style="display:flex; flex-direction:column; gap:.5rem; justify-content:center;">
          <button class="btn-secondary btn-sm" :disabled="actionPending" @click="setDatetime">
            <i class="fa-solid fa-pen-to-square"></i> {{ t('time.setDatetime') }}
          </button>
          <button class="btn-secondary btn-sm" :disabled="actionPending" @click="changeTimezone">
            <i class="fa-solid fa-globe"></i> {{ t('time.changeTimezone') }}
          </button>
          <button class="btn-secondary btn-sm" :disabled="actionPending" @click="syncNow">
            <i class="fa-solid fa-arrows-rotate"></i> {{ t('time.syncNow') }}
          </button>
        </div>
      </div>
    </div>

    <!-- ── NTP ────────────────────────────────────────────────── -->
    <div class="card" style="margin-top:16px;">
      <div class="flex" style="align-items:center; justify-content:space-between; margin-bottom:1rem;">
        <h2 style="margin:0;">{{ t('time.ntp') }}</h2>
        <div class="btn-group">
          <button v-if="!ntpEnabled" class="btn-secondary btn-sm" :disabled="actionPending" @click="ntpEnable">
            <i class="fa-solid fa-play"></i> {{ t('common.start') }}
          </button>
          <button v-if="ntpEnabled" class="btn-secondary btn-sm" :disabled="actionPending" @click="ntpDisable">
            <i class="fa-solid fa-stop"></i> {{ t('common.stop') }}
          </button>
          <button v-if="ntpEnabled" class="btn-secondary btn-sm" :disabled="actionPending" @click="ntpRestart">
            <i class="fa-solid fa-rotate-right"></i> {{ t('common.restart') }}
          </button>
        </div>
      </div>

      <!-- Condensed status row -->
      <div style="display:flex; gap:1rem; flex-wrap:wrap; align-items:center; margin-bottom:1rem;">
        <span v-if="ntpRunning" class="badge badge-success">{{ t('common.running') }}</span>
        <span v-else-if="ntpEnabled" class="badge badge-warn">{{ t('common.stopped') }}</span>
        <span v-else class="badge badge-dim">{{ t('common.disabled') }}</span>
        <span v-if="ntpRunning && status.ntp.stratum != null" class="text-dim">{{ t('time.stratum') }}: {{ status.ntp.stratum }}</span>
        <span v-if="ntpRunning && status.ntp.offset_ms != null" class="text-dim">{{ t('time.offset') }}: {{ status.ntp.offset_ms.toFixed(2) }} ms</span>
        <span v-if="ntpRunning && status.ntp.system_peer" class="text-dim">{{ t('time.systemPeer') }}: <span class="mono">{{ status.ntp.system_peer }}</span></span>
      </div>

      <!-- Sync-on-start toggle -->
      <div v-if="ntpConf" style="margin-bottom:1rem;">
        <label class="checkbox-label">
          <input type="checkbox" :checked="ntpConf.sync_on_start" @change="toggleSyncOnStart" />
          <span>{{ t('time.syncOnStart') }}</span>
          <span class="text-dim">— {{ t('time.syncOnStartHint') }}</span>
        </label>
      </div>

      <div class="btn-group">
        <button class="btn-secondary btn-sm" @click="showServersModal = true">
          <i class="fa-solid fa-server"></i> {{ t('time.ntpServers') }}
        </button>
        <button class="btn-secondary btn-sm" @click="showPeersModal = true">
          <i class="fa-solid fa-table-list"></i> {{ t('time.ntpPeers') }}
        </button>
      </div>
    </div>

    <!-- Servers modal -->
    <div v-if="showServersModal" class="modal-overlay">
      <div class="modal modal-wide">
        <h3>{{ t('time.ntpServers') }}</h3>

        <div v-if="editServers.length" style="margin-bottom:.5rem;">
          <div v-for="(s, i) in editServers" :key="i" class="ntp-server-row">
            <span class="ntp-pill" :class="s.kind === 'pool' ? 'ntp-pill-pool' : 'ntp-pill-server'">{{ s.kind }}</span>
            <input v-model="s.host" class="ntp-host mono" placeholder="ntp.example.com" @input="onHostInput(s)" />
            <span v-if="s.options.includes('iburst')" class="ntp-pill ntp-pill-iburst">iburst</span>
            <button class="btn-secondary btn-sm" @click="removeServer(i)" :title="t('common.delete')">
              <i class="fa-solid fa-xmark"></i>
            </button>
          </div>
        </div>
        <div v-else class="text-dim" style="margin-bottom:.5rem;">{{ t('time.noServers') }}</div>

        <div class="modal-actions" style="justify-content:flex-start;">
          <button class="btn-secondary btn-sm" @click="addServer">
            <i class="fa-solid fa-plus"></i> {{ t('time.addServer') }}
          </button>
          <button class="btn-primary btn-sm" :disabled="saving" @click="saveServers">
            <i class="fa-solid fa-floppy-disk"></i> {{ t('time.saveServers') }}
          </button>
          <button type="button" class="btn-secondary" @click="showServersModal = false">{{ t('common.close') }}</button>
        </div>
      </div>
    </div>
    <!-- Peers modal -->
    <div v-if="showPeersModal" class="modal-overlay">
      <div class="modal modal-wide">
        <h3>{{ t('time.ntpPeers') }}</h3>
        <div v-if="ntpRunning && status.ntp.peers.length">
          <table>
            <thead><tr>
              <th>{{ t('time.peerRemote') }}</th>
              <th>{{ t('time.peerRefid') }}</th>
              <th>{{ t('time.peerStratum') }}</th>
              <th>{{ t('time.peerDelay') }}</th>
              <th>{{ t('time.peerOffset') }}</th>
              <th>{{ t('time.peerJitter') }}</th>
              <th>{{ t('time.peerState') }}</th>
            </tr></thead>
            <tbody>
              <tr v-for="(p, i) in status.ntp.peers" :key="i">
                <td class="mono">{{ p.remote }}</td>
                <td class="mono">{{ p.refid }}</td>
                <td>{{ p.stratum }}</td>
                <td>{{ p.delay_ms.toFixed(2) }}</td>
                <td>{{ p.offset_ms.toFixed(2) }}</td>
                <td>{{ p.jitter_ms.toFixed(2) }}</td>
                <td>
                  <span v-if="p.state === 'sync'" class="badge badge-success">{{ t('time.stateSync') }}</span>
                  <span v-else-if="p.state === 'candidate'" class="badge badge-info">{{ t('time.stateCandidate') }}</span>
                  <span v-else-if="p.state === 'outlier'" class="badge badge-dim">{{ t('time.stateOutlier') }}</span>
                  <span v-else-if="p.state === 'false'" class="badge badge-warn">{{ t('time.stateFalse') }}</span>
                  <span v-else-if="p.state === 'backup'" class="badge badge-info">{{ t('time.stateBackup') }}</span>
                  <span v-else class="badge badge-dim">—</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="text-dim">{{ t('time.noNtpData') }}</div>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" @click="showPeersModal = false">{{ t('common.close') }}</button>
        </div>
      </div>
    </div>
  </template>
</template>

<style scoped>
.time-label {
  font-size: .8rem;
  color: var(--text-dim, #888);
  margin-bottom: .25rem;
}
.time-value {
  font-size: 1.5rem;
  font-weight: 600;
  letter-spacing: .5px;
}
.time-sub {
  font-size: .8rem;
  color: var(--text-dim, #888);
  margin-top: .25rem;
  display: flex;
  align-items: center;
}
.ntp-server-row {
  display: grid;
  grid-template-columns: 4.5rem 1fr 4rem 2rem;
  gap: .5rem;
  align-items: center;
  margin-bottom: .5rem;
}
.ntp-pill {
  padding: .15rem .3rem;
  border-radius: 999px;
  font-size: .72rem;
  font-weight: 600;
  line-height: 1.4;
  display: block;
  text-align: center;
  box-sizing: border-box;
  width: 100%;
}
.ntp-pill-pool {
  background: rgba(74, 144, 217, .15);
  color: #6da8dc;
  border: 1px solid rgba(74, 144, 217, .35);
}
.ntp-pill-server {
  background: rgba(120, 180, 100, .12);
  color: #8fbc7a;
  border: 1px solid rgba(120, 180, 100, .3);
}
.ntp-pill-iburst {
  background: rgba(200, 150, 60, .12);
  color: #cba35e;
  border: 1px solid rgba(200, 150, 60, .3);
}
.ntp-host {
  flex: 1;
  min-width: 120px;
}
</style>
