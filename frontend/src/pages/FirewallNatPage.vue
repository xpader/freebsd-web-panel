<script setup>
import { ref, computed, onMounted } from 'vue';
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
const interfaces = ref([]);
const defaultIface = ref('');
const loading = ref(true);
const rulesLoading = ref(false);

const initialized = computed(() => status.value?.initialized);
const isIpfw = computed(() => status.value?.driver === 'ipfw');

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
    rules.value = await api.get('/api/firewall/nat/rules');
  } catch (e) {
    rules.value = [];
  } finally {
    rulesLoading.value = false;
  }
}

async function loadInterfaces() {
  // Reuse /api/network/gateway to discover the default-route egress interface
  // (typical SNAT target). Falls back to empty if unavailable.
  try {
    const gw = await api.get('/api/network/gateway');
    defaultIface.value = gw.interface || '';
  } catch (e) { /* ignore */ }
  try {
    const ifaces = await api.get('/api/network/interfaces');
    interfaces.value = (ifaces || []).map(i => i.name).filter(Boolean);
  } catch (e) {
    interfaces.value = defaultIface.value ? [defaultIface.value] : [];
  }
}

async function loadAll() {
  await loadStatus();
  await loadRules();
}

function makeFields(rule = null) {
  const ifaceOptions = interfaces.value.map(name => ({ value: name, label: name }));
  const defaultIfaceVal = rule?.interface || defaultIface.value || (interfaces.value[0] || '');

  return [
    { key: 'description', label: t('common.description'), value: rule?.description || '', placeholder: 'NAT for jail network' },
    {
      key: 'kind', label: t('common.type'), type: 'radio', value: rule?.kind || 'snat',
      options: [
        { value: 'snat', label: t('firewall.natKindSnat') },
        { value: 'dnat', label: t('firewall.natKindDnat') },
      ],
    },
    {
      key: 'family', label: t('firewall.natFamily'), type: 'radio', value: rule?.family || 'ip',
      options: [
        { value: 'ip', label: 'IPv4' },
        { value: 'ip6', label: 'IPv6' },
      ],
    },
    {
      key: 'protocol', label: t('firewall.protocol'), type: 'select', value: rule?.protocol || 'both', half: true, row: 'proto-iface',
      options: [
        { value: 'both', label: t('common.all') },
        { value: 'tcp', label: 'TCP' },
        { value: 'udp', label: 'UDP' },
      ],
    },
    {
      key: 'interface', label: t('firewall.interface'), type: 'select', value: defaultIfaceVal, half: true, row: 'proto-iface',
      options: ifaceOptions,
    },
    // SNAT: internal network to NAT (required)
    {
      key: 'src_addr', label: t('firewall.natSrcAddr'),
      value: rule?.src_addr || '',
      placeholder: '10.0.0.0/24',
      showIf: { kind: 'snat' },
      requiredIf: { kind: 'snat' },
    },
    // DNAT: source IP (optional alias IPs) + source port (required) — same row
    {
      key: 'src_addr', label: t('firewall.natSrcIp'),
      value: rule?.src_addr === 'any' ? '' : (rule?.src_addr || ''),
      placeholder: t('firewall.natSrcIpPh'),
      hint: t('firewall.natSrcIpHint'),
      half: true, row: 'dnat-src',
      showIf: { kind: 'dnat' },
    },
    {
      key: 'src_port', label: t('firewall.srcPort'),
      value: rule?.src_port || '', placeholder: '80', half: true, row: 'dnat-src',
      showIf: { kind: 'dnat' },
      requiredIf: { kind: 'dnat' },
    },
    // DNAT: internal target IP (required) + target port (optional) — same row
    {
      key: 'dst_addr', label: t('firewall.natDstAddr'),
      value: rule?.dst_addr || '', placeholder: '10.0.0.2', half: true, row: 'dnat-target',
      showIf: { kind: 'dnat' },
      requiredIf: { kind: 'dnat' },
    },
    {
      key: 'dst_port', label: t('firewall.natTargetPort'),
      value: rule?.dst_port || '', placeholder: '8080', half: true, row: 'dnat-target',
      showIf: { kind: 'dnat' },
    },
  ];
}

function extractBody(result) {
  const body = {
    kind: result.kind,
    family: result.family,
    interface: result.interface,
    src_addr: result.kind === 'dnat' ? (result.src_addr || 'any') : (result.src_addr || ''),
    protocol: result.protocol,
    enabled: true,
    description: result.description || null,
  };
  if (result.dst_addr) body.dst_addr = result.dst_addr;
  if (result.kind === 'dnat') {
    body.src_port = result.src_port || null;
    body.dst_port = result.dst_port || null;
  }
  return body;
}

async function doAddRule() {
  if (!interfaces.value.length) await loadInterfaces();
  await formModal(t('firewall.addNatTitle'), makeFields(), {
    submitLabel: t('common.create'),
    submitHandler: async (r) => {
      await api.post('/api/firewall/nat/rules', extractBody(r));
      toast.toast(t('firewall.natRuleAdded'));
      await loadStatus();
      await loadRules();
    },
  });
}

async function doEditRule(rule) {
  if (!interfaces.value.length) await loadInterfaces();
  await formModal(t('firewall.editNatTitle'), makeFields(rule), {
    submitLabel: t('common.save'),
    submitHandler: async (r) => {
      await api.put(`/api/firewall/nat/rules/${rule.id}`, extractBody(r));
      toast.toast(t('firewall.natRuleUpdated'));
      await loadStatus();
      await loadRules();
    },
  });
}

async function doCopyRule(rule) {
  if (!interfaces.value.length) await loadInterfaces();
  await formModal(t('firewall.addNatTitle'), makeFields({ ...rule, description: null }), {
    submitLabel: t('common.create'),
    submitHandler: async (r) => {
      await api.post('/api/firewall/nat/rules', extractBody(r));
      toast.toast(t('firewall.natRuleAdded'));
      await loadStatus();
      await loadRules();
    },
  });
}

async function doDeleteRule(rule) {
  if (!await confirm(t('firewall.deleteNatTitle'), t('firewall.deleteNatConfirm'))) return;
  try {
    await api.del(`/api/firewall/nat/rules/${rule.id}`);
    toast.toast(t('firewall.natRuleDeleted'));
    await loadStatus();
    await loadRules();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || t('common.deleteFailed'));
  }
}

async function doToggleRule(rule) {
  try {
    await api.put(`/api/firewall/nat/rules/${rule.id}/toggle`);
    await loadStatus();
    await loadRules();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

function kindLabel(kind) {
  const map = {
    snat: t('firewall.natKindSnat'),
    dnat: t('firewall.natKindDnat'),
    binat: t('firewall.natKindBinat'),
  };
  return map[kind] || kind;
}

function ruleSummary(rule) {
  if (rule.kind === 'snat') {
    const target = rule.dst_addr || `(${rule.interface})`;
    return `${rule.src_addr}  →  ${target}`;
  }
  if (rule.kind === 'dnat') {
    const target = rule.dst_addr || '?';
    const tport = rule.dst_port ? `:${rule.dst_port}` : '';
    const port = rule.src_port || '';
    return `port ${port}  →  ${target}${tport}`;
  }
  if (rule.kind === 'binat') {
    return `${rule.src_addr}  ↔  ${rule.dst_addr || '?'}`;
  }
  return '';
}

function protoLabel(p) {
  if (p === 'both') return t('common.all');
  return p.toUpperCase();
}

onMounted(() => {
  loadAll();
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <h1>{{ t('firewall.natRulesTitle') }}</h1>
      <p class="text-dim" style="margin:0;font-size:13px;">{{ t('firewall.natSubtitle') }}</p>
    </div>
    <div v-if="initialized" class="flex btn-group" style="margin-left:auto;">
      <button @click="doAddRule">
        <i class="fa-solid fa-plus"></i> {{ t('firewall.addNatRule') }}
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
    <div class="card empty">
      <p class="text-dim">{{ t('firewall.initDesc') }}</p>
      <router-link to="/firewall/settings" class="btn-secondary" style="display:inline-flex;">
        {{ t('firewall.settings') }}
      </router-link>
    </div>
  </template>

  <template v-else>
    <div v-if="status.pending_apply" class="card" style="padding:12px 16px;">
      <div class="flex" style="align-items:center;gap:12px;">
        <i class="fa-solid fa-triangle-exclamation" style="color:var(--warn);"></i>
        <span class="text-dim">{{ t('firewall.pendingApply') }}</span>
        <router-link to="/firewall/rules" style="margin-left:auto;">
          <button class="btn-sm">{{ t('firewall.applyRules') }}</button>
        </router-link>
      </div>
    </div>

    <div v-if="isIpfw" class="card" style="padding:8px 16px;">
      <div class="flex" style="align-items:center;gap:8px;">
        <i class="fa-solid fa-circle-info" style="color:var(--info);"></i>
        <span class="text-dim" style="font-size:13px;">{{ t('firewall.natIpfwNatModuleHint') }}</span>
      </div>
    </div>

    <div class="card" style="padding:0;">
      <table>
        <thead>
          <tr>
            <th>#</th>
            <th>{{ t('common.enabled') }}</th>
            <th>{{ t('common.type') }}</th>
            <th>{{ t('firewall.protocol') }}</th>
            <th>{{ t('firewall.interface') }}</th>
            <th>{{ t('firewall.natFamily') }}</th>
            <th>{{ t('common.description') }}</th>
            <th>{{ t('common.actions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="rulesLoading">
            <td colspan="8" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td>
          </tr>
          <tr v-else-if="!rules.length">
            <td colspan="8" class="empty">{{ t('firewall.noNatRules') }}</td>
          </tr>
          <tr v-for="(rule, idx) in rules" :key="rule.id" :class="{ 'row-disabled': !rule.enabled }">
            <td class="mono">{{ idx + 1 }}</td>
            <td>
              <span :class="['badge', rule.enabled ? 'badge-success' : 'badge-muted']" @click="doToggleRule(rule)" style="cursor:pointer;">
                {{ rule.enabled ? t('common.enabled') : t('common.disabled') }}
              </span>
            </td>
            <td><span :class="['badge', rule.kind === 'snat' ? 'badge-dim' : rule.kind === 'dnat' ? 'badge-success' : 'badge-warn']">{{ kindLabel(rule.kind) }}</span></td>
            <td>{{ protoLabel(rule.protocol) }}</td>
            <td class="mono">{{ rule.interface }}</td>
            <td>{{ rule.family === 'ip' ? 'IPv4' : 'IPv6' }}</td>
            <td>
              <div class="cell-wrap"><span v-if="rule.description">{{ rule.description }}</span><span v-else class="text-dim mono">{{ ruleSummary(rule) }}</span></div>
            </td>
            <td>
              <div class="btn-group">
                <button class="btn-secondary btn-sm" @click="doEditRule(rule)">{{ t('common.edit') }}</button>
                <button class="btn-secondary btn-sm" @click="doCopyRule(rule)">{{ t('common.copy') }}</button>
                <button class="btn-danger btn-sm" @click="doDeleteRule(rule)">{{ t('common.delete') }}</button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>

<style scoped>
.row-disabled {
  opacity: 0.5;
}
</style>
