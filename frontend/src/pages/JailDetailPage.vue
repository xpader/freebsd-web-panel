<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import SectionCard from '../components/ui/SectionCard.vue';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;

const d = ref(null);
const error = ref('');
const activeTab = ref('overview');

const running = computed(() => d.value?.jid > 0);
const merged = computed(() => {
  if (!d.value) return {};
  const m = { ...(d.value.params || {}) };
  if (d.value.runtime) {
    for (const [k, v] of Object.entries(d.value.runtime.params || {})) m[k] = v;
  }
  return m;
});
const rt = computed(() => d.value?.runtime);
const ip4Addr = computed(() => {
  if (rt.value?.ip4_addr?.length) return rt.value.ip4_addr;
  if (merged.value['ip4.addr']) return merged.value['ip4.addr'].split(',').map((s) => s.trim());
  return [];
});
const ip6Addr = computed(() => {
  if (rt.value?.ip6_addr?.length) return rt.value.ip6_addr;
  if (merged.value['ip6.addr']) return merged.value['ip6.addr'].split(',').map((s) => s.trim());
  return [];
});
const persist = computed(() => merged.value.persist === 'true');
const allowEntries = computed(() => Object.entries(merged.value).filter(([k]) => k.startsWith('allow.')).sort(([a], [b]) => a.localeCompare(b)));
const otherEntries = computed(() => Object.entries(merged.value).filter(([k]) => !k.startsWith('allow.')).sort(([a], [b]) => a.localeCompare(b)));

const tabItems = computed(() => {
  const tabs = [
    { key: 'overview', label: t('jails.basicInfo') },
    { key: 'network', label: t('common.network') },
    { key: 'exec', label: t('jails.editExec') },
    { key: 'security', label: t('jails.security') },
    { key: 'permissions', label: t('common.permissions') },
    { key: 'all', label: t('jails.allParams') },
  ];
  if (rt.value) tabs.splice(4, 0, { key: 'runtime', label: t('jails.runtimeInfo') });
  return tabs;
});

function stateBadge(state) {
  if (state === 'running') return { cls: 'badge-success', text: t('jails.running') };
  if (state === 'dying') return { cls: 'badge-warn', text: t('jails.dying') };
  return { cls: 'badge-dim', text: t('jails.stopped') };
}

onMounted(async () => {
  try {
    d.value = await api.get(`/api/jails/${encodeURIComponent(name)}`);
  } catch (err) {
    error.value = err.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/jails/running" />
      <h1>{{ name }}</h1>
    </div>
    <div class="flex btn-group" style="margin-left:auto;">
        <a v-if="running" :href="`#/jails/terminal/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-terminal"></i> {{ t('term.openTerminal') }}</a>
        <a :href="`#/jails/edit/${name}`" class="btn-secondary btn-sm"><i class="fa-solid fa-pen-to-square"></i> {{ t('common.edit') }}</a>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!d" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <!-- Status bar -->
    <div class="card">
      <div class="flex" style="flex-wrap:wrap;gap:16px;align-items:center;">
        <div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">JID</span><strong class="mono">{{ d.jid || '—' }}</strong></div>
        <div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">{{ t('common.status') }}</span><span :class="['badge', stateBadge(running ? (rt?.state || 'running') : 'stopped').cls]">{{ stateBadge(running ? (rt?.state || 'running') : 'stopped').text }}</span></div>
        <div class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">persist</span>
          <span v-if="persist" class="badge badge-success">{{ t('common.enabled') }}</span>
          <span v-else class="badge badge-dim">{{ t('common.disabled') }}</span>
        </div>
        <div v-if="merged.parent" class="flex" style="gap:6px;align-items:center;"><span class="text-dim" style="font-size:12px;">{{ t('jails.parent') }}</span><strong class="mono">{{ merged.parent }}</strong></div>
      </div>
    </div>

    <SectionCard v-model="activeTab" :tabs="tabItems">
      <template #default="{ active }">
      <!-- Overview -->
      <table v-if="active === 'overview'" class="kv-table">
        <tbody>
        <tr><td class="mono text-dim">{{ t('jails.autoStart') }}</td><td>
          <i v-if="d.auto_start" class="fa-solid fa-check" style="color: var(--success);"></i>
          <i v-else class="fa-solid fa-xmark" style="color: var(--danger);"></i>
        </td></tr>
        <tr><td class="mono text-dim">path</td><td class="mono">{{ merged.path || '—' }}</td></tr>
        <tr><td class="mono text-dim">host.hostname</td><td class="mono">{{ merged['host.hostname'] || d.name || '—' }}</td></tr>
        <tr><td class="mono text-dim">host.domainname</td><td class="mono">{{ merged['host.domainname'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">host.hostuuid</td><td class="mono">{{ merged['host.hostuuid'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">host.hostid</td><td class="mono">{{ merged['host.hostid'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">mount.fstab</td><td class="mono">{{ merged['mount.fstab'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">mount.devfs</td><td class="mono">{{ merged['mount.devfs'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">mount.fdescfs</td><td class="mono">{{ merged['mount.fdescfs'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">mount.procfs</td><td class="mono">{{ merged['mount.procfs'] || '—' }}</td></tr>
        </tbody>
      </table>

      <!-- Network -->
      <table v-else-if="active === 'network'" class="kv-table">
        <tbody>
        <tr><td class="mono text-dim">interface</td><td class="mono">{{ merged.interface || '—' }}</td></tr>
        <tr><td class="mono text-dim">ip4</td><td class="mono">{{ merged.ip4 || '—' }}</td></tr>
        <tr><td class="mono text-dim">ip4.addr</td><td class="mono">{{ ip4Addr.length ? ip4Addr.join(', ') : '—' }}</td></tr>
        <tr><td class="mono text-dim">ip6</td><td class="mono">{{ merged.ip6 || '—' }}</td></tr>
        <tr><td class="mono text-dim">ip6.addr</td><td class="mono">{{ ip6Addr.length ? ip6Addr.join(', ') : '—' }}</td></tr>
        <tr><td class="mono text-dim">vnet</td><td class="mono">{{ merged.vnet || '—' }}</td></tr>
        </tbody>
      </table>

      <!-- Execution -->
      <table v-else-if="active === 'exec'" class="kv-table">
        <tbody>
        <tr><td class="mono text-dim">exec.start</td><td class="mono">{{ merged['exec.start'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.stop</td><td class="mono">{{ merged['exec.stop'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.clean</td><td class="mono">{{ merged['exec.clean'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.jail_user</td><td class="mono">{{ merged['exec.jail_user'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.system_user</td><td class="mono">{{ merged['exec.system_user'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.prestart</td><td class="mono">{{ merged['exec.prestart'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.poststop</td><td class="mono">{{ merged['exec.poststop'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.timeout</td><td class="mono">{{ merged['exec.timeout'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">exec.consolelog</td><td class="mono">{{ merged['exec.consolelog'] || '—' }}</td></tr>
        </tbody>
      </table>

      <!-- Security -->
      <table v-else-if="active === 'security'" class="kv-table">
        <tbody>
        <tr><td class="mono text-dim">securelevel</td><td class="mono">{{ merged.securelevel || '—' }}</td></tr>
        <tr><td class="mono text-dim">enforce_statfs</td><td class="mono">{{ merged.enforce_statfs || '—' }}</td></tr>
        <tr><td class="mono text-dim">devfs_ruleset</td><td class="mono">{{ merged.devfs_ruleset || '—' }}</td></tr>
        <tr><td class="mono text-dim">children.max</td><td class="mono">{{ merged['children.max'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">children.cur</td><td class="mono">{{ merged['children.cur'] || '—' }}</td></tr>
        </tbody>
      </table>

      <!-- Runtime -->
      <table v-else-if="active === 'runtime'" class="kv-table">
        <tbody>
        <tr><td class="mono text-dim">jid</td><td class="mono">{{ d.jid }}</td></tr>
        <tr><td class="mono text-dim">osrelease</td><td class="mono">{{ merged.osrelease || '—' }}</td></tr>
        <tr><td class="mono text-dim">osreldate</td><td class="mono">{{ merged.osreldate || '—' }}</td></tr>
        <tr><td class="mono text-dim">cpuset.id</td><td class="mono">{{ merged['cpuset.id'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">dying</td><td class="mono">{{ merged.dying || 'false' }}</td></tr>
        <tr><td class="mono text-dim">ip4.saddrsel</td><td class="mono">{{ merged['ip4.saddrsel'] || '—' }}</td></tr>
        <tr><td class="mono text-dim">ip6.saddrsel</td><td class="mono">{{ merged['ip6.saddrsel'] || '—' }}</td></tr>
        </tbody>
      </table>

      <!-- Permissions -->
      <div v-else-if="active === 'permissions'" class="perm-grid">
        <span v-for="[k, v] in allowEntries" :key="k" :class="['badge', (v === 'true' || v === '1') ? 'badge-success' : 'badge-dim']">{{ k.replace(/^allow\./, '') }}</span>
      </div>

      <!-- All params -->
      <table v-else-if="active === 'all'" class="kv-table">
        <tbody>
        <tr v-for="[k, v] in otherEntries" :key="k">
          <td class="mono text-dim">{{ k }}</td><td class="mono">{{ v || '—' }}</td>
        </tr>
        </tbody>
      </table>
      </template>
    </SectionCard>
  </template>
</template>
