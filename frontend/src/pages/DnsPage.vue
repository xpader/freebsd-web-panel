<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const alert = useAlert();

const cfg = ref(null);
const servers = ref(['', '', '']);
const loading = ref(true);
const error = ref('');

function isValidIp(s) {
  if (/^(\d{1,3}\.){3}\d{1,3}$/.test(s)) {
    return s.split('.').every((o) => Number(o) >= 0 && Number(o) <= 255);
  }
  return /^[0-9a-fA-F:]+$/.test(s) && s.includes(':');
}

function clearSlot(i) {
  servers.value[i] = '';
}

function swap(i, dir) {
  const j = i + dir;
  if (j < 0 || j > 2) return;
  [servers.value[i], servers.value[j]] = [servers.value[j], servers.value[i]];
}

async function apply() {
  for (const s of servers.value) {
    if (s && !isValidIp(s)) {
      await alert(t('common.operationFailed'), t('dns.invalidIp', { addr: s }));
      return;
    }
  }
  const filled = servers.value.filter((s) => s);
  if (new Set(filled).size !== filled.length) {
    await alert(t('common.operationFailed'), t('dns.duplicate'));
    return;
  }
  try {
    cfg.value = await api.put('/api/network/dns/nameservers', { nameservers: servers.value });
    servers.value = [0, 1, 2].map((i) => cfg.value.nameservers[i] || '');
    toast.toast(t('common.saved'));
  } catch (e) {
    await alert(t('common.saveFailed', { msg: '' }), e.message || t('common.saveFailed', { msg: '' }));
  }
}

onMounted(async () => {
  try {
    cfg.value = await api.get('/api/network/dns');
    servers.value = [0, 1, 2].map((i) => cfg.value.nameservers[i] || '');
  } catch (err) {
    error.value = err.message || '';
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('dns.title') }}</h1>
    <p>{{ t('dns.subtitle') }}</p>
  </div>
  <div class="toolbar">
    <span></span>
    <div class="flex">
      <button @click="apply"><i class="fa-solid fa-check"></i> {{ t('common.apply') }}</button>
    </div>
  </div>

  <div v-if="error" class="card" style="padding:1rem;">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="loading" class="card" style="padding:1rem;"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <template v-else-if="cfg">
    <div class="card">
      <div class="dns-slots">
        <div v-for="i in 3" :key="i" class="dns-slot">
          <label class="dns-slot-label">NameServer {{ i }}</label>
          <div class="dns-slot-input">
            <input
              type="text"
              v-model="servers[i - 1]"
              class="dns-input mono"
              :placeholder="i === 1 ? '8.8.8.8' : ''"
            />
            <button class="btn-secondary btn-sm dns-slot-clear" :title="t('common.clear')" @click="clearSlot(i - 1)">
              <i class="fa-solid fa-xmark"></i>
            </button>
          </div>
          <button class="btn-secondary btn-sm" :disabled="i === 1" :title="t('net.up')" @click="swap(i - 1, -1)"><i class="fa-solid fa-arrow-up"></i></button>
          <button class="btn-secondary btn-sm" :disabled="i === 3" :title="t('net.down')" @click="swap(i - 1, 1)"><i class="fa-solid fa-arrow-down"></i></button>
        </div>
      </div>
    </div>

    <div v-if="cfg.domain || cfg.search.length || cfg.options.length" class="card" style="margin-top:16px;">
      <div v-if="cfg.domain" class="kv"><span class="kv-key">{{ t('dns.domain') }}</span><span class="kv-val mono">{{ cfg.domain }}</span></div>
      <div v-if="cfg.search.length" class="kv"><span class="kv-key">{{ t('dns.search') }}</span><span class="kv-val mono">{{ cfg.search.join(', ') }}</span></div>
      <div v-if="cfg.options.length" class="kv"><span class="kv-key">{{ t('dns.options') }}</span><span class="kv-val mono">{{ cfg.options.join(', ') }}</span></div>
    </div>
  </template>
</template>
