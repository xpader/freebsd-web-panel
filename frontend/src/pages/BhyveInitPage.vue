<script setup>
import { ref, computed } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const initType = ref('zfs');
const zfsDataset = ref('');
const dirPath = ref('');
const initializing = ref(false);
const steps = ref([]);
const error = ref('');

const spec = computed(() => {
  if (initType.value === 'zfs') {
    return zfsDataset.value.trim() ? `zfs:${zfsDataset.value.trim()}` : '';
  }
  return dirPath.value.trim();
});

const canSubmit = computed(() => {
  if (initializing.value) return false;
  if (initType.value === 'zfs') return zfsDataset.value.trim().length > 0;
  return dirPath.value.trim().startsWith('/');
});

async function doInit() {
  if (!canSubmit.value) return;
  initializing.value = true;
  error.value = '';
  steps.value = [];
  try {
    const result = await api.post('/api/bhyve/init', { spec: spec.value });
    steps.value = result;
    toast.toast(t('bhyve.initSuccess'));
    setTimeout(() => router.push('/bhyve/vms'), 2000);
  } catch (e) {
    error.value = e.message || '';
    await alert(t('bhyve.initFailed'), error.value);
  } finally {
    initializing.value = false;
  }
}
</script>

<template>
  <div class="page-header">
    <h1>{{ t('bhyve.initTitle') }}</h1>
    <p>{{ t('bhyve.initDesc') }}</p>
  </div>

  <div class="card">
    <h3>{{ t('bhyve.initStorageType') }}</h3>
    <div class="flex" style="gap:24px;margin-bottom:16px;">
      <label class="flex" style="gap:6px;cursor:pointer;align-items:center;">
        <input type="radio" value="zfs" v-model="initType" :disabled="initializing" />
        <span>ZFS {{ t('bhyve.initDataset') }}</span>
      </label>
      <label class="flex" style="gap:6px;cursor:pointer;align-items:center;">
        <input type="radio" value="directory" v-model="initType" :disabled="initializing" />
        <span>{{ t('bhyve.initDirectory') }}</span>
      </label>
    </div>

    <div v-if="initType === 'zfs'" class="field">
      <label>{{ t('bhyve.initZfsDataset') }}</label>
      <input
        v-model="zfsDataset"
        :placeholder="t('bhyve.initZfsPlaceholder')"
        :disabled="initializing"
      />
      <p class="text-dim" style="font-size:12px;margin-top:4px;">{{ t('bhyve.initZfsHint') }}</p>
    </div>

    <div v-else class="field">
      <label>{{ t('bhyve.initDirPath') }}</label>
      <input
        v-model="dirPath"
        :placeholder="t('bhyve.initDirPlaceholder')"
        :disabled="initializing"
      />
      <p class="text-dim" style="font-size:12px;margin-top:4px;">{{ t('bhyve.initDirHint') }}</p>
    </div>

    <div v-if="initializing" style="margin:16px 0;">
      <span class="spinner"></span> {{ t('bhyve.initializing') }}
    </div>

    <div v-if="error" class="alert-error" style="margin:16px 0;">
      <strong>{{ t('common.operationFailed') }}</strong>: {{ error }}
    </div>

    <div v-if="steps.length" style="margin:16px 0;">
      <h4>{{ t('bhyve.initSteps') }}</h4>
      <ul style="margin:8px 0;padding-left:20px;">
        <li v-for="(s, i) in steps" :key="i" class="mono" style="font-size:13px;margin:4px 0;">
          <i class="fa-solid fa-check" style="color:var(--success);"></i> {{ s }}
        </li>
      </ul>
    </div>

    <div class="btn-group">
      <button @click="doInit" :disabled="!canSubmit">
        <i class="fa-solid fa-rocket"></i> {{ t('bhyve.initStart') }}
      </button>
      <button class="btn-secondary" @click="router.push('/bhyve/vms')" :disabled="initializing">
        {{ t('common.cancel') }}
      </button>
    </div>
  </div>

  <div class="card">
    <h3>{{ t('bhyve.initWhatHappens') }}</h3>
    <ol style="margin:8px 0;padding-left:20px;line-height:1.8;">
      <li>{{ t('bhyve.initStep1') }}</li>
      <li>{{ t('bhyve.initStep2') }}</li>
      <li>{{ t('bhyve.initStep3') }}</li>
      <li>{{ t('bhyve.initStep4') }}</li>
      <li>{{ t('bhyve.initStep5') }}</li>
    </ol>
  </div>
</template>
