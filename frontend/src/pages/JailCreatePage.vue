<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import FilePicker from '../components/ui/FilePicker.vue';
import SectionCard from '../components/ui/SectionCard.vue';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const bases = ref([]);
const submitting = ref(false);
const form = ref({
  name: '', hostname: '', location_type: '', interface: '',
  ip4: '', ip6: '',
  dir_path: '', base_name: '', snapshot: '',
  target_dataset: '', base_path: '', sfs_path: '',
  auto_start: false,
});

const sections = computed(() => [
  { key: 'basic', label: t('jails.basicInfo') },
  { key: 'location', label: t('jails.locationType') },
  { key: 'network', label: t('common.network') },
]);

const selectedBase = computed(() => bases.value.find((b) => b.name === form.value.base_name));
const isZfsBase = computed(() => selectedBase.value?.type === 'zfs');

const pickerTarget = ref(null);
const pickerConfig = ref({ mode: 'dir', accept: [] });

function openPicker(target, mode = 'dir', accept = []) {
  pickerTarget.value = target;
  pickerConfig.value = { mode, accept };
}
function onPickerSelect(path) {
  if (pickerTarget.value) form.value[pickerTarget.value] = path;
  pickerTarget.value = null;
}

async function onSubmit() {
  const result = {
    name: form.value.name,
    hostname: form.value.hostname || null,
    location_type: form.value.location_type,
    interface: form.value.interface || null,
    ip4: form.value.ip4 || null,
    ip6: form.value.ip6 || null,
    auto_start: form.value.auto_start,
  };

  if (form.value.location_type === 'directory') {
    result.path = form.value.dir_path;
  } else if (form.value.location_type === 'base') {
    result.base_name = form.value.base_name;
    const base = selectedBase.value;
    if (base?.type === 'zfs') {
      result.snapshot = form.value.snapshot;
      result.target_dataset = form.value.target_dataset || null;
      result.path = form.value.base_path || null;
    } else if (base?.type === 'sharedfs') {
      result.path = form.value.sfs_path || null;
    }
  }

  submitting.value = true;
  try {
    await api.post('/api/jails/create', result);
    toast.toast(t('jails.jailCreated'));
    router.push('/jails/running');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    submitting.value = false;
  }
}

onMounted(async () => {
  try { bases.value = await api.get('/api/jails/bases'); } catch {}
});
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <a href="#/jails/running" class="btn-secondary btn-sm">{{ t('common.navBack') }}</a>
      <h1>{{ t('jails.createTitle') }}</h1>
    </div>
  </div>

  <form @submit.prevent="onSubmit">
    <SectionCard :tabs="sections" expand>
      <template #default="{ active }">

      <!-- Basic Info -->
      <template v-if="active === 'basic'">
        <div class="form-row">
          <label class="form-row-label">{{ t('jails.jailName') }} <span style="color:var(--danger)">*</span></label>
          <div>
            <input type="text" v-model="form.name" required placeholder="web01" />
            <p class="param-desc">{{ t('jails.descJailName') }}</p>
          </div>
        </div>
        <div class="form-row">
          <label class="form-row-label">{{ t('jails.hostname') }}</label>
          <div>
            <input type="text" v-model="form.hostname" :placeholder="t('jails.hostnamePh')" />
            <p class="param-desc">{{ t('jails.descHostname') }}</p>
          </div>
        </div>
        <div class="form-row">
          <label class="form-row-label">{{ t('jails.autoStart') }}</label>
          <div>
            <label class="checkbox-label">
              <input type="checkbox" v-model="form.auto_start" />
              <span class="param-desc-inline">{{ t('jails.descAutoStart') }}</span>
            </label>
          </div>
        </div>
      </template>

      <!-- Location -->
      <template v-if="active === 'location'">
        <div class="form-row">
          <label class="form-row-label">{{ t('jails.locationType') }} <span style="color:var(--danger)">*</span></label>
          <div>
            <select v-model="form.location_type" required>
              <option value="">{{ t('common.pleaseSelect') }}</option>
              <option value="directory">{{ t('jails.locDirectory') }}</option>
              <option value="base">{{ t('jails.locBase') }}</option>
            </select>
          </div>
        </div>

        <div v-if="form.location_type === 'directory'">
          <div class="form-row">
            <label class="form-row-label">{{ t('jails.path') }} <span style="color:var(--danger)">*</span></label>
            <div>
              <div class="input-with-btn">
                <input type="text" v-model="form.dir_path" :placeholder="form.name ? `/jails/${form.name}` : '/jails/<jailname>'" />
                <button type="button" class="btn-secondary btn-sm" @click="openPicker('dir_path')"><i class="fa-solid fa-folder-open"></i></button>
              </div>
              <p class="param-desc">{{ t('jails.descDirPath') }}</p>
            </div>
          </div>
        </div>

        <div v-if="form.location_type === 'base'">
          <div class="form-row">
            <label class="form-row-label">{{ t('jails.selectBase') }} <span style="color:var(--danger)">*</span></label>
            <div>
              <select v-model="form.base_name">
                <option value="">{{ t('common.pleaseSelect') }}</option>
                <option v-for="b in bases" :key="b.name" :value="b.name">{{ b.name }} ({{ b.type }})</option>
              </select>
            </div>
          </div>

          <div v-if="selectedBase && isZfsBase">
            <div class="form-row">
              <label class="form-row-label">{{ t('jails.cloneSnapshot') }} <span style="color:var(--danger)">*</span></label>
              <div>
                <select v-model="form.snapshot">
                  <option value="">{{ t('common.pleaseSelect') }}</option>
                  <option v-for="s in (selectedBase.snapshots || [])" :key="s" :value="s">{{ s.includes('@') ? s.split('@').pop() : s }}</option>
                </select>
              </div>
            </div>
            <div class="form-row">
              <label class="form-row-label">{{ t('jails.targetDataset') }}</label>
              <div>
                <input type="text" v-model="form.target_dataset" :placeholder="form.name ? `zroot/jails/${form.name}` : 'zroot/jails/<jailname>'" />
                <p class="param-desc">{{ t('jails.descTargetDataset') }}</p>
              </div>
            </div>
            <div class="form-row">
              <label class="form-row-label">{{ t('jails.mountPoint') }}</label>
              <div>
                <div class="input-with-btn">
                  <input type="text" v-model="form.base_path" :placeholder="form.name ? `/jails/${form.name}` : '/jails/<jailname>'" />
                  <button type="button" class="btn-secondary btn-sm" @click="openPicker('base_path')"><i class="fa-solid fa-folder-open"></i></button>
                </div>
              </div>
            </div>
          </div>

          <div v-if="selectedBase && !isZfsBase">
            <div class="form-row">
              <label class="form-row-label">{{ t('jails.targetLocation') }}</label>
              <div>
                <div class="input-with-btn">
                  <input type="text" v-model="form.sfs_path" :placeholder="form.name ? `/jails/${form.name}` : '/jails/<jailname>'" />
                  <button type="button" class="btn-secondary btn-sm" @click="openPicker('sfs_path')"><i class="fa-solid fa-folder-open"></i></button>
                </div>
                <p class="param-desc">{{ t('jails.descSfsPath') }}</p>
              </div>
            </div>
          </div>
        </div>
      </template>

      <!-- Network -->
      <template v-if="active === 'network'">
        <div class="form-row">
          <label class="form-row-label">{{ t('jails.networkInterface') }}</label>
          <div>
            <input type="text" v-model="form.interface" placeholder="bge0" />
            <p class="param-desc">{{ t('jails.descInterface') }}</p>
          </div>
        </div>
        <div class="form-row">
          <label class="form-row-label">IPv4</label>
          <div>
            <input type="text" v-model="form.ip4" :placeholder="t('jails.ip4Ph')" />
            <p class="param-desc">{{ t('jails.descIpAddr') }}</p>
          </div>
        </div>
        <div class="form-row">
          <label class="form-row-label">IPv6</label>
          <div>
            <input type="text" v-model="form.ip6" :placeholder="t('jails.ip6Ph')" />
          </div>
        </div>
      </template>

      </template>
    </SectionCard>

    <div class="form-actions-bar">
      <a href="#/jails/running" class="btn btn-secondary">{{ t('common.cancel') }}</a>
      <button type="submit" :disabled="submitting">{{ t('common.confirm') }}</button>
    </div>
  </form>

  <FilePicker
    v-if="pickerTarget"
    :mode="pickerConfig.mode"
    :accept="pickerConfig.accept"
    :initial-path="form[pickerTarget] || '/'"
    @select="onPickerSelect"
    @close="pickerTarget = null"
  />
</template>

<style scoped>
.form-row-label {
  padding-top: 8px;
}
.param-desc {
  font-size: 12px;
  color: var(--text-dim);
  margin: 4px 0 0 0;
}
.param-desc-inline {
  font-size: 12px;
  color: var(--text-dim);
}
.checkbox-label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  padding-top: 8px;
}
</style>
