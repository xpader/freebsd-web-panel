<script setup>
import { ref, computed, onMounted, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();

const tabs = computed(() => [
  { key: 'global', label: 'jail.conf', icon: 'fa-solid fa-file-lines', api: '/api/jails/config/global' },
  { key: 'devfs', label: 'devfs.rules', icon: 'fa-solid fa-shield-halved', api: '/api/jails/config/devfs' },
  { key: 'resolv', label: 'resolv.conf', icon: 'fa-solid fa-globe', api: '/api/jails/config/resolv' },
]);

const activeTab = ref('global');
const content = ref('');
const loading = ref(true);
const saving = ref(false);
const error = ref('');

const currentTab = computed(() => tabs.value.find(tb => tb.key === activeTab.value));
const titleKey = computed(() => `jails.defaults.${activeTab.value}.title`);
const subtitleKey = computed(() => `jails.defaults.${activeTab.value}.subtitle`);

async function load() {
  loading.value = true;
  error.value = '';
  try {
    const data = await api.get(currentTab.value.api);
    content.value = data.content || '';
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
}

async function save() {
  saving.value = true;
  try {
    await api.put(currentTab.value.api, { content: content.value });
    toast.toast(t('common.saved'));
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  } finally {
    saving.value = false;
  }
}

function switchTab(key) {
  if (key === activeTab.value) return;
  activeTab.value = key;
  load();
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <h1>{{ t('nav.jailDefaults') }}</h1>
    <p>{{ t('jails.defaults.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <div class="filter-group">
      <button
        v-for="tb in tabs"
        :key="tb.key"
        :class="['filter-btn', { active: activeTab === tb.key }]"
        @click="switchTab(tb.key)"
      >
        <i :class="tb.icon"></i> {{ tb.label }}
      </button>
    </div>
    <div class="flex">
      <button @click="save" :disabled="saving || loading">
        <i :class="['fa-solid fa-check', { 'fa-spin': saving }]"></i> {{ t('common.save') }}
      </button>
    </div>
  </div>
  <p class="text-dim defaults-tab-desc">{{ t(subtitleKey) }}</p>
  <div v-if="error" class="card" style="padding:1rem;">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="loading" class="card" style="padding:1rem;"><span class="spinner"></span> {{ t('common.loading') }}</div>
  <div v-else class="card" style="padding:0;">
    <textarea
      v-model="content"
      class="config-editor"
      spellcheck="false"
    ></textarea>
  </div>
</template>

<style scoped>
.config-editor {
  width: 100%;
  min-height: 500px;
  padding: 16px;
  border: none;
  border-radius: var(--radius);
  background: var(--bg-elev);
  color: var(--text);
  font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
  font-size: 14px;
  line-height: 1.6;
  resize: vertical;
  outline: none;
  tab-size: 8;
}
.config-editor:focus {
  box-shadow: inset 0 0 0 1px var(--accent);
}
.defaults-tab-desc {
  margin: 4px 0 12px;
  font-size: 13px;
}
</style>
