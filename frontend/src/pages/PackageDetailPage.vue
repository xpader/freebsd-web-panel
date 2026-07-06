<script setup>
import { ref, onMounted } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes, fmtDate } from '../lib/format.js';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;
const info = ref(null);
const error = ref('');
const activeTab = ref('info');
const files = ref(null);

async function loadFiles() {
  try {
    files.value = await api.get(`/api/pkg/packages/${encodeURIComponent(name)}/files`);
  } catch (e) {
    files.value = [];
  }
}

function setTab(tab) {
  activeTab.value = tab;
  if (tab === 'files' && !files.value) loadFiles();
}

onMounted(async () => {
  try {
    info.value = await api.get(`/api/pkg/packages/${encodeURIComponent(name)}`);
  } catch (e) {
    error.value = e.message || '';
  }
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/pkg" />
      <h1>{{ info ? `${info.name}-${info.version}` : name }}</h1>
    </div>
    <p>{{ t('pkg.detailSubtitle') }}</p>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="!info" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else>
    <div class="toolbar" style="margin-bottom:16px;">
      <div class="filter-group">
        <button :class="['filter-btn', { active: activeTab === 'info' }]" @click="setTab('info')">{{ t('pkg.tabInfo') }}</button>
        <button :class="['filter-btn', { active: activeTab === 'deps' }]" @click="setTab('deps')">{{ t('pkg.tabDeps') }}</button>
        <button :class="['filter-btn', { active: activeTab === 'files' }]" @click="setTab('files')">{{ t('pkg.tabFiles') }}</button>
      </div>
    </div>

    <!-- Info tab -->
    <div v-if="activeTab === 'info'" class="card">
      <div class="flex" style="justify-content:space-between; margin-bottom:12px;">
        <div><strong style="font-size:18px;">{{ info.name }}-{{ info.version }}</strong></div>
        <div>
          <span v-if="info.automatic" class="badge badge-dim">{{ t('pkg.automatic') }}</span>
          <span v-else class="badge badge-success">{{ t('pkg.manual') }}</span>
          <span v-if="info.locked" class="badge badge-warn">{{ t('pkg.locked') }}</span>
          <span v-if="info.vital" class="badge badge-success">{{ t('pkg.vital') }}</span>
        </div>
      </div>
      <p v-if="info.comment" style="margin-bottom:16px; font-size:15px; color:var(--text-dim);">{{ info.comment }}</p>
      <div v-if="info.description" style="margin-bottom:16px;">
        <div class="card-title">{{ t('common.description') }}</div>
        <p style="white-space:pre-wrap;">{{ info.description }}</p>
      </div>
      <table class="kv-table">
        <tbody>
        <tr><td>{{ t('pkg.origin') }}</td><td class="mono">{{ info.origin }}</td></tr>
        <tr><td>{{ t('pkg.version') }}</td><td class="mono">{{ info.version }}</td></tr>
        <tr><td>{{ t('common.size') }}</td><td>{{ fmtBytes(info.size_bytes) }}</td></tr>
        <tr><td>{{ t('pkg.prefix') }}</td><td class="mono">{{ info.prefix }}</td></tr>
        <tr><td>{{ t('pkg.homepage') }}</td><td><a :href="info.homepage" target="_blank" rel="noopener">{{ info.homepage }}</a></td></tr>
        <tr><td>{{ t('pkg.maintainer') }}</td><td>{{ info.maintainer }}</td></tr>
        <tr><td>{{ t('pkg.repository') }}</td><td>{{ info.repository }}</td></tr>
        <tr><td>ABI</td><td class="mono">{{ info.abi }}</td></tr>
        <tr><td>{{ t('pkg.arch') }}</td><td class="mono">{{ info.arch }}</td></tr>
        <tr><td>{{ t('pkg.installed') }}</td><td>{{ fmtDate(info.install_timestamp) }}</td></tr>
        </tbody>
      </table>
    </div>

    <!-- Deps tab -->
    <div v-else-if="activeTab === 'deps'" style="display:grid; grid-template-columns: 1fr 1fr; gap:16px;">
      <div class="card">
        <div class="card-title">{{ t('pkg.dependsOn') }} ({{ (info.dependencies || []).length }})</div>
        <div v-if="(info.dependencies || []).length">
          <table>
            <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('pkg.version') }}</th></tr></thead>
            <tbody>
              <tr v-for="d in (info.dependencies || [])" :key="d.name" style="cursor:pointer;" @click="$router.push('/pkg/' + d.name)">
                <td class="mono">{{ d.name }}</td>
                <td class="mono text-dim">{{ d.version }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="empty">{{ t('pkg.noDeps') }}</div>
      </div>
      <div class="card">
        <div class="card-title">{{ t('pkg.requiredBy') }} ({{ (info.reverse_dependencies || []).length }})</div>
        <div v-if="(info.reverse_dependencies || []).length">
          <table>
            <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('pkg.version') }}</th></tr></thead>
            <tbody>
              <tr v-for="d in (info.reverse_dependencies || [])" :key="d.name" style="cursor:pointer;" @click="$router.push('/pkg/' + d.name)">
                <td class="mono">{{ d.name }}</td>
                <td class="mono text-dim">{{ d.version }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="empty">{{ t('pkg.noRdeps') }}</div>
      </div>
    </div>

    <!-- Files tab -->
    <div v-else class="card" style="padding:0;">
      <div v-if="!files" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
      <div v-else-if="!files.length" class="empty">{{ t('pkg.noFiles') }}</div>
      <table v-else>
        <thead><tr>
          <th>{{ t('pkg.filePath') }}</th><th>{{ t('common.owner') }}</th><th>{{ t('common.group') }}</th><th>{{ t('common.permissions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-for="(f, i) in files" :key="i">
            <td class="mono">{{ f.path }}</td>
            <td>{{ f.owner || '—' }}</td>
            <td>{{ f.group || '—' }}</td>
            <td class="mono">{{ f.mode || '—' }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
