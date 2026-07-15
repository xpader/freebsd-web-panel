<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const status = ref(null);
const rules = ref([]);
const configPreview = ref('');
const loading = ref(true);
const rulesLoading = ref(false);
const configLoading = ref(false);

const initialized = computed(() => status.value?.initialized);

async function loadStatus() {
  try {
    status.value = await api.get('/api/firewall/status');
  } catch (e) {
    if (!status.value) loading.value = false;
    return;
  }
  loading.value = false;
}

async function loadRules() {
  if (!initialized.value) return;
  rulesLoading.value = true;
  try {
    rules.value = await api.get('/api/firewall/rules');
  } catch (e) {
    rules.value = [];
  } finally {
    rulesLoading.value = false;
  }
}

async function loadConfig() {
  if (!initialized.value) return;
  configLoading.value = true;
  try {
    const data = await api.get('/api/firewall/config');
    configPreview.value = data.content;
  } catch (e) {
    configPreview.value = '';
  } finally {
    configLoading.value = false;
  }
}

async function loadAll() {
  await loadStatus();
  await Promise.all([loadRules(), loadConfig()]);
}

async function doInitialize() {
  const result = await formModal(t('firewall.initTitle'), [
    {
      key: 'driver',
      label: t('firewall.driver'),
      type: 'radio',
      options: [
        { value: 'ipfw', label: 'ipfw' },
        { value: 'pf', label: 'pf' },
      ],
      value: 'ipfw',
    },
    {
      key: 'mode',
      label: t('firewall.mode'),
      type: 'radio',
      options: [
        { value: 'whitelist', label: t('firewall.whitelist') },
        { value: 'blacklist', label: t('firewall.blacklist') },
      ],
      value: 'blacklist',
    },
  ], { submitLabel: t('firewall.initialize') });

  if (!result) return;

  try {
    await api.post('/api/firewall/initialize', { driver: result.driver, mode: result.mode });
    toast.toast(t('firewall.initialized'));
    await loadAll();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doToggleEnabled() {
  if (status.value.enabled) {
    if (!await confirm(t('firewall.disableTitle'), t('firewall.disableConfirm'))) return;
    try {
      status.value = await api.post('/api/firewall/disable');
      toast.toast(t('firewall.disabled'));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
  } else {
    try {
      status.value = await api.post('/api/firewall/enable');
      toast.toast(t('firewall.enabled'));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
  }
}

async function doSettings() {
  const curDriver = status.value.driver;
  const curMode = status.value.mode;
  const result = await formModal(t('firewall.settings'), [
    {
      key: 'driver',
      label: t('firewall.driver'),
      type: 'radio',
      options: [
        { value: 'ipfw', label: 'ipfw' },
        { value: 'pf', label: 'pf' },
      ],
      value: curDriver,
    },
    {
      key: 'mode',
      label: t('firewall.mode'),
      type: 'radio',
      options: [
        { value: 'whitelist', label: t('firewall.whitelist') },
        { value: 'blacklist', label: t('firewall.blacklist') },
      ],
      value: curMode,
    },
  ], { submitLabel: t('common.save') });

  if (!result) return;

  // Switch driver if changed
  if (result.driver && result.driver !== curDriver) {
    if (!await confirm(t('firewall.switchConfirmTitle'),
        t('firewall.switchConfirm', { from: curDriver, to: result.driver }))) return;
    try {
      status.value = await api.post('/api/firewall/switch', { driver: result.driver });
      toast.toast(t('firewall.switched', { driver: result.driver }));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
      return;
    }
  }

  // Switch mode if changed
  const targetMode = result.mode;
  if (targetMode && targetMode !== curMode) {
    const warnMsg = targetMode === 'whitelist'
      ? t('firewall.modeSwitchWhitelistWarn')
      : t('firewall.modeSwitchBlacklistWarn');
    if (!await confirm(t('firewall.modeConfirmTitle'), warnMsg)) return;
    try {
      status.value = await api.put('/api/firewall/mode', { mode: targetMode });
      toast.toast(t('firewall.modeSwitched'));
    } catch (e) {
      await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
  }

  await loadAll();
}

function makeFields(rule = null) {
  return [
    { key: 'description', label: t('common.description'), value: rule?.description || '', placeholder: 'Allow HTTP' },
    {
      key: 'action', label: t('firewall.action'), type: 'select', value: rule?.action || 'allow', half: true,
      options: [
        { value: 'allow', label: t('firewall.allow') },
        { value: 'deny', label: t('firewall.deny') },
        { value: 'reject', label: t('firewall.reject') },
      ],
    },
    {
      key: 'direction', label: t('firewall.direction'), type: 'select', value: rule?.direction || 'in', half: true,
      options: [
        { value: 'in', label: t('firewall.inbound') },
        { value: 'out', label: t('firewall.outbound') },
      ],
    },
    {
      key: 'protocol', label: t('firewall.protocol'), type: 'select', value: rule?.protocol || 'tcp', half: true,
      options: [
        { value: 'tcp', label: 'TCP' },
        { value: 'udp', label: 'UDP' },
        { value: 'icmp', label: 'ICMP' },
        { value: 'icmpv6', label: 'ICMPv6' },
        { value: 'any', label: t('common.all') },
      ],
    },
    {
      key: 'log', label: t('firewall.logging'), type: 'select', value: rule ? (rule.log ? '1' : '0') : '0', half: true,
      options: [
        { value: '0', label: t('common.no') },
        { value: '1', label: t('common.yes') },
      ],
    },
    {
      key: 'icmpType', label: 'ICMP Type', type: 'select', value: rule?.icmp_type || '',
      options: [
        { value: '', label: t('common.all') },
        { value: 'echo-request', label: 'echo-request (8) Ping' },
        { value: 'echo-reply', label: 'echo-reply (0) Ping Reply' },
        { value: 'destination-unreachable', label: 'destination-unreachable (3)' },
        { value: 'source-quench', label: 'source-quench (4)' },
        { value: 'redirect', label: 'redirect (5)' },
        { value: 'time-exceeded', label: 'time-exceeded (11) Traceroute' },
        { value: 'parameter-problem', label: 'parameter-problem (12)' },
        { value: 'timestamp', label: 'timestamp (13)' },
        { value: 'timestamp-reply', label: 'timestamp-reply (14)' },
      ],
      showIf: { protocol: ['icmp'] },
    },
    {
      key: 'icmpv6Type', label: 'ICMPv6 Type', type: 'select', value: rule?.icmp_type || '',
      options: [
        { value: '', label: t('common.all') },
        { value: 'echo-request', label: 'echo-request (128) Ping6' },
        { value: 'echo-reply', label: 'echo-reply (129) Ping6 Reply' },
        { value: 'destination-unreachable', label: 'destination-unreachable (1)' },
        { value: 'packet-too-big', label: 'packet-too-big (2)' },
        { value: 'time-exceeded', label: 'time-exceeded (3)' },
        { value: 'parameter-problem', label: 'parameter-problem (4)' },
        { value: 'router-solicitation', label: 'router-solicitation (133)' },
        { value: 'router-advertisement', label: 'router-advertisement (134)' },
        { value: 'neighbor-solicitation', label: 'neighbor-solicitation (135)' },
        { value: 'neighbor-advertisement', label: 'neighbor-advertisement (136)' },
        { value: 'redirect', label: 'redirect (137)' },
      ],
      showIf: { protocol: ['icmpv6'] },
    },
    {
      key: 'srcKind', label: t('firewall.source'), type: 'select', value: rule?.source?.kind || 'any', half: true,
      options: [
        { value: 'any', label: t('firewall.addrAny') },
        { value: 'me', label: t('firewall.addrMe') },
        { value: 'single', label: t('firewall.addrSingle') },
        { value: 'cidr', label: t('firewall.addrCidr') },
      ],
    },
    {
      key: 'srcValue', label: t('firewall.addrValue'), value: rule?.source?.value || '', half: true,
      placeholder: '192.168.1.1',
      showIf: { srcKind: ['single'] },
      requiredIf: { srcKind: ['single'] },
    },
    {
      key: 'srcValueCidr', label: t('firewall.addrValue'), value: rule?.source?.value || '', half: true,
      placeholder: '10.0.0.0/24',
      showIf: { srcKind: ['cidr'] },
      requiredIf: { srcKind: ['cidr'] },
    },
    {
      key: 'srcPort', label: t('firewall.srcPort'), value: rule?.source_port || '', half: true,
      placeholder: '80 or 1024-65535 or 53,80,443-450',
      showIf: { protocol: ['tcp', 'udp'] },
    },
    {
      key: 'dstKind', label: t('firewall.destination'), type: 'select', value: rule?.destination?.kind || 'me', half: true,
      options: [
        { value: 'any', label: t('firewall.addrAny') },
        { value: 'me', label: t('firewall.addrMe') },
        { value: 'single', label: t('firewall.addrSingle') },
        { value: 'cidr', label: t('firewall.addrCidr') },
      ],
    },
    {
      key: 'dstValue', label: t('firewall.addrValue'), value: rule?.destination?.value || '', half: true,
      placeholder: '192.168.1.1',
      showIf: { dstKind: ['single'] },
      requiredIf: { dstKind: ['single'] },
    },
    {
      key: 'dstValueCidr', label: t('firewall.addrValue'), value: rule?.destination?.value || '', half: true,
      placeholder: '10.0.0.0/24',
      showIf: { dstKind: ['cidr'] },
      requiredIf: { dstKind: ['cidr'] },
    },
    {
      key: 'dstPort', label: t('firewall.dstPort'), value: rule?.destination_port || '', half: true,
      placeholder: '80 or 443,8080 or 80,443,8080-8090',
      showIf: { protocol: ['tcp', 'udp'] },
    },
    { key: 'interface', label: t('firewall.interface'), value: rule?.interface || '', placeholder: 'em0 (optional)' },
  ];
}

function extractBody(result) {
  const srcKind = result.srcKind;
  const dstKind = result.dstKind;
  const proto = result.protocol;

  const srcValue = srcKind === 'single' ? (result.srcValue || '') : srcKind === 'cidr' ? (result.srcValueCidr || '') : '';
  const dstValue = dstKind === 'single' ? (result.dstValue || '') : dstKind === 'cidr' ? (result.dstValueCidr || '') : '';

  let srcPort = null, dstPort = null;
  if (proto === 'tcp' || proto === 'udp') {
    srcPort = result.srcPort || null;
    dstPort = result.dstPort || null;
  }

  let icmpType = null;
  if (proto === 'icmp') {
    icmpType = result.icmpType || null;
  } else if (proto === 'icmpv6') {
    icmpType = result.icmpv6Type || null;
  }

  return {
    action: result.action,
    direction: result.direction,
    protocol: proto,
    source: { kind: srcKind, value: srcValue },
    source_port: srcPort,
    destination: { kind: dstKind, value: dstValue },
    destination_port: dstPort,
    interface: result.interface || null,
    log: result.log === '1',
    icmp_type: icmpType,
    description: result.description || null,
  };
}

async function doAddRule() {
  const result = await formModal(t('firewall.addRuleTitle'), makeFields(), { submitLabel: t('common.create') });
  if (!result) return;
  try {
    await api.post('/api/firewall/rules', extractBody(result));
    toast.toast(t('firewall.ruleAdded'));
    status.value.pending_apply = true;
    await Promise.all([loadRules(), loadConfig()]);
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doEditRule(rule) {
  const result = await formModal(t('firewall.editRuleTitle'), makeFields(rule), { submitLabel: t('common.save') });
  if (!result) return;
  try {
    await api.put(`/api/firewall/rules/${rule.id}`, extractBody(result));
    toast.toast(t('firewall.ruleUpdated'));
    status.value.pending_apply = true;
    await Promise.all([loadRules(), loadConfig()]);
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doDeleteRule(rule) {
  if (!await confirm(t('firewall.deleteRuleTitle'), t('firewall.deleteRuleConfirm'))) return;
  try {
    await api.del(`/api/firewall/rules/${rule.id}`);
    toast.toast(t('firewall.ruleDeleted'));
    status.value.pending_apply = true;
    await Promise.all([loadRules(), loadConfig()]);
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

async function doToggleRule(rule) {
  try {
    await api.put(`/api/firewall/rules/${rule.id}/toggle`);
    status.value.pending_apply = true;
    await Promise.all([loadRules(), loadConfig()]);
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doMoveRule(index, direction) {
  const newIdx = index + direction;
  if (newIdx < 0 || newIdx >= rules.value.length) return;
  const ids = rules.value.map(r => r.id);
  [ids[index], ids[newIdx]] = [ids[newIdx], ids[index]];
  try {
    await api.put('/api/firewall/rules/reorder', { ordered_ids: ids });
    status.value.pending_apply = true;
    await Promise.all([loadRules(), loadConfig()]);
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function doApply() {
  try {
    await api.post('/api/firewall/apply');
    toast.toast(t('firewall.applied'));
    status.value.pending_apply = false;
    await loadStatus();
    await loadConfig();
  } catch (e) {
    await alert(t('firewall.applyFailed'), e.message || t('firewall.applyFailed'));
  }
}

function actionLabel(action) {
  const map = { allow: t('firewall.allow'), deny: t('firewall.deny'), reject: t('firewall.reject') };
  return map[action] || action;
}

function addrLabel(spec) {
  if (!spec) return '\u2014';
  if (spec.kind === 'any') return t('firewall.addrAny');
  if (spec.kind === 'me') return t('firewall.addrMe');
  return spec.value || '\u2014';
}

function protoLabel(p) {
  if (p === 'any') return t('common.all');
  return p.toUpperCase();
}

let pollTimer = null;
onMounted(() => {
  loadAll();
  pollTimer = setInterval(loadStatus, 5000);
});
onUnmounted(() => clearInterval(pollTimer));
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <h1>{{ t('firewall.title') }}</h1>
      <p class="text-dim" style="margin:0;font-size:13px;">{{ t('firewall.subtitle') }}</p>
    </div>
    <div v-if="initialized" class="flex btn-group" style="margin-left:auto;">
      <button :class="status.enabled ? 'btn-danger' : ''" @click="doToggleEnabled">
        <i :class="status.enabled ? 'fa-solid fa-stop' : 'fa-solid fa-play'"></i>
        {{ status.enabled ? t('common.stop') : t('common.start') }}
      </button>
      <button class="btn-secondary" @click="doSettings">
        <i class="fa-solid fa-gear"></i> {{ t('firewall.settings') }}
      </button>
      <button class="btn-secondary" @click="loadAll">
        <i class="fa-solid fa-rotate"></i> {{ t('common.refresh') }}
      </button>
    </div>
  </div>

  <div v-if="loading" class="card">
    <div class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
  </div>

  <template v-else-if="!initialized">
    <div class="card firewall-init">
      <h3>{{ t('firewall.initTitle') }}</h3>
      <p class="text-dim">{{ t('firewall.initDesc') }}</p>
      <div class="firewall-driver-compare">
        <div class="driver-card">
          <h4>ipfw</h4>
          <ul class="text-dim">
            <li>{{ t('firewall.ipfwFeature1') }}</li>
            <li>{{ t('firewall.ipfwFeature2') }}</li>
            <li>{{ t('firewall.ipfwFeature3') }}</li>
          </ul>
        </div>
        <div class="driver-card">
          <h4>pf</h4>
          <ul class="text-dim">
            <li>{{ t('firewall.pfFeature1') }}</li>
            <li>{{ t('firewall.pfFeature2') }}</li>
            <li>{{ t('firewall.pfFeature3') }}</li>
          </ul>
        </div>
      </div>
      <div class="modal-actions" style="justify-content:center;">
        <button @click="doInitialize"><i class="fa-solid fa-shield-halved"></i> {{ t('firewall.initialize') }}</button>
      </div>
    </div>
  </template>

  <template v-else>
    <div class="card">
      <div class="flex" style="flex-wrap:wrap;gap:16px;align-items:center;">
        <div class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('firewall.driver') }}</span>
          <strong class="mono">{{ status.driver }}</strong>
        </div>
        <div class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('firewall.mode') }}</span>
          <strong>{{ status.mode === 'whitelist' ? t('firewall.whitelist') : t('firewall.blacklist') }}</strong>
        </div>
        <div class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('common.status') }}</span>
          <span :class="['badge', status.enabled ? 'badge-success' : 'badge-warn']">
            {{ status.enabled ? t('firewall.running') : t('firewall.stopped') }}
          </span>
        </div>
        <div class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('firewall.moduleLoaded') }}</span>
          <span :class="['badge', status.module_loaded ? 'badge-success' : 'badge-warn']">
            {{ status.module_loaded ? t('common.yes') : t('common.no') }}
          </span>
        </div>
        <div class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('firewall.rulesCount') }}</span>
          <strong class="mono">{{ status.rules_count }}</strong>
        </div>
        <div v-if="status.pending_apply" class="flex" style="gap:6px;align-items:center;">
          <span class="text-dim" style="font-size:12px;">{{ t('firewall.pendingApply') }}</span>
          <span class="badge badge-warn">{{ t('common.yes') }}</span>
        </div>
      </div>
    </div>

    <div class="card" style="padding:0;">
      <div class="toolbar" style="padding:12px 16px;">
        <h3 style="margin:0;">{{ t('firewall.rulesTitle') }}</h3>
        <div class="flex"></div>
        <div class="btn-group">
          <button @click="doApply" v-if="status.pending_apply">
            <i class="fa-solid fa-check"></i> {{ t('firewall.applyRules') }}
          </button>
          <button @click="doAddRule">
            <i class="fa-solid fa-plus"></i> {{ t('firewall.addRule') }}
          </button>
        </div>
      </div>
      <table>
        <thead>
          <tr>
            <th>#</th>
            <th>{{ t('common.enabled') }}</th>
            <th>{{ t('firewall.action') }}</th>
            <th>{{ t('firewall.direction') }}</th>
            <th>{{ t('firewall.protocol') }}</th>
            <th>{{ t('firewall.source') }}</th>
            <th>{{ t('firewall.destination') }}</th>
            <th>{{ t('common.description') }}</th>
            <th>{{ t('common.actions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="rulesLoading">
            <td colspan="9" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td>
          </tr>
          <tr v-else-if="!rules.length">
            <td colspan="9" class="empty">{{ t('firewall.noRules') }}</td>
          </tr>
          <tr v-for="(rule, idx) in rules" :key="rule.id" :class="{ 'row-disabled': !rule.enabled }">
            <td class="mono">{{ idx + 1 }}</td>
            <td>
              <span :class="['badge', rule.enabled ? 'badge-success' : 'badge-muted']" @click="doToggleRule(rule)" style="cursor:pointer;">
                {{ rule.enabled ? t('common.enabled') : t('common.disabled') }}
              </span>
            </td>
            <td><span :class="['badge', rule.action === 'allow' ? 'badge-success' : 'badge-danger']">{{ actionLabel(rule.action) }}</span></td>
            <td>{{ rule.direction === 'in' ? t('firewall.inbound') : t('firewall.outbound') }}</td>
            <td>{{ protoLabel(rule.protocol) }}<span v-if="rule.icmp_type" class="text-dim"> ({{ rule.icmp_type }})</span></td>
            <td class="mono">
              {{ addrLabel(rule.source) }}<span v-if="rule.source_port" class="text-dim">:{{ rule.source_port }}</span>
            </td>
            <td class="mono">
              {{ addrLabel(rule.destination) }}<span v-if="rule.destination_port" class="text-dim">:{{ rule.destination_port }}</span>
            </td>
            <td><div class="cell-wrap">{{ rule.description || '\u2014' }}</div></td>
            <td>
              <div class="btn-group">
                <button class="btn-secondary btn-sm" @click="doMoveRule(idx, -1)" :disabled="idx === 0" title="Up">
                  <i class="fa-solid fa-arrow-up"></i>
                </button>
                <button class="btn-secondary btn-sm" @click="doMoveRule(idx, 1)" :disabled="idx === rules.length - 1" title="Down">
                  <i class="fa-solid fa-arrow-down"></i>
                </button>
                <button class="btn-secondary btn-sm" @click="doEditRule(rule)">{{ t('common.edit') }}</button>
                <button class="btn-danger btn-sm" @click="doDeleteRule(rule)">{{ t('common.delete') }}</button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div class="card">
      <div class="toolbar" style="padding:0 0 12px 0;">
        <h3 style="margin:0;">{{ t('firewall.configPreview') }}</h3>
        <div class="flex"></div>
        <span class="text-dim">{{ status.driver === 'ipfw' ? '/etc/ipfw.rules' : '/etc/pf.conf' }}</span>
      </div>
      <pre class="config-preview mono" v-if="configPreview">{{ configPreview }}</pre>
      <div v-else class="empty">{{ configLoading ? t('common.loading') : t('common.noData') }}</div>
    </div>
  </template>
</template>

<style scoped>
.firewall-init {
  text-align: center;
  padding: 32px;
}
.firewall-init h3 {
  margin-bottom: 8px;
}
.firewall-init > p {
  margin-bottom: 24px;
}
.firewall-driver-compare {
  display: flex;
  gap: 16px;
  margin-bottom: 24px;
}
.driver-card {
  flex: 1;
  text-align: left;
  padding: 16px;
  border-radius: var(--radius);
  background: var(--bg-elev2);
}
.driver-card h4 {
  margin: 0 0 8px 0;
}
.driver-card ul {
  margin: 0;
  padding-left: 20px;
  font-size: 13px;
  line-height: 1.8;
}
.row-disabled {
  opacity: 0.5;
}
.config-preview {
  background: var(--bg-elev2);
  padding: 12px 16px;
  border-radius: var(--radius);
  font-size: 13px;
  line-height: 1.5;
  overflow-x: auto;
  max-height: 400px;
  overflow-y: auto;
}
</style>
