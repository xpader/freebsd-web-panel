<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import BackButton from '../components/ui/BackButton.vue';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const templates = ref([]);
const datastores = ref([]);
const switches = ref([]);
const submitting = ref(false);

const form = ref({
  name: '',
  template: 'default',
  datastore: '',
  size: '20G',
  cpu: 1,
  memory: '512M',
  switch_name: 'public',
  start_after: false,
  install_iso: '',
});

const isos = ref([]);

async function loadOptions() {
  try {
    const [tmpls, ds, sws, isoList] = await Promise.all([
      api.get('/api/bhyve/templates'),
      api.get('/api/bhyve/datastores'),
      api.get('/api/bhyve/switches'),
      api.get('/api/bhyve/isos'),
    ]);
    templates.value = tmpls;
    datastores.value = ds;
    switches.value = sws;
    isos.value = isoList;
    if (ds.length) form.value.datastore = ds[0].name;
  } catch {
    // ignore — selectors will be empty
  }
}

const selectedTemplate = computed(() => form.value.template);

async function onSubmit() {
  if (!form.value.name) {
    await alert(t('common.operationFailed'), t('bhyve.errNameRequired'));
    return;
  }
  if (!form.value.template) {
    await alert(t('common.operationFailed'), t('bhyve.errTemplateRequired'));
    return;
  }

  const body = {
    name: form.value.name,
    template: form.value.template,
    datastore: form.value.datastore || null,
    size: form.value.size || null,
    cpu: Number(form.value.cpu) || null,
    memory: form.value.memory || null,
  };

  submitting.value = true;
  try {
    await api.post('/api/bhyve/vms', body);
    if (form.value.install_iso && form.value.start_after) {
      try {
        await api.post(`/api/bhyve/vms/${encodeURIComponent(form.value.name)}/install`, {
          iso: form.value.install_iso,
        });
      } catch {
        // install failure is non-fatal — VM was created
      }
    } else if (form.value.start_after) {
      try {
        await api.post(`/api/bhyve/vms/${encodeURIComponent(form.value.name)}/start`);
      } catch {
        // start failure is non-fatal
      }
    }
    toast.toast(t('bhyve.vmCreated', { name: form.value.name }));
    router.push('/bhyve/vms');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
    submitting.value = false;
  }
}

onMounted(loadOptions);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton href="#/bhyve/vms" />
      <h1>{{ t('bhyve.createVm') }}</h1>
    </div>
  </div>

  <form @submit.prevent="onSubmit">
    <div class="card" style="padding:24px;">
      <!-- Name -->
      <div class="form-row">
        <label class="form-row-label">{{ t('bhyve.vmName') }} <span style="color:var(--danger)">*</span></label>
        <div>
          <input type="text" v-model="form.name" required placeholder="my-vm"
            pattern="[a-z0-9][a-z0-9._-]*[a-z0-9]" />
          <p class="param-desc">{{ t('bhyve.descVmName') }}</p>
        </div>
      </div>

      <!-- Template -->
      <div class="form-row">
        <label class="form-row-label">{{ t('bhyve.template') }} <span style="color:var(--danger)">*</span></label>
        <div>
          <select v-model="form.template" required>
            <option v-for="tp in templates" :key="tp" :value="tp">{{ tp }}</option>
          </select>
          <p class="param-desc">{{ t('bhyve.descTemplate') }}</p>
        </div>
      </div>

      <!-- Datastore -->
      <div class="form-row">
        <label class="form-row-label">{{ t('bhyve.datastore') }}</label>
        <div>
          <select v-model="form.datastore">
            <option v-for="ds in datastores" :key="ds.name" :value="ds.name">
              {{ ds.name }} ({{ ds.type }})
            </option>
          </select>
        </div>
      </div>

      <!-- CPU / Memory / Disk -->
      <div class="form-grid-3">
        <div class="form-row">
          <label class="form-row-label">CPU</label>
          <div>
            <input type="number" v-model.number="form.cpu" min="1" max="64" />
          </div>
        </div>
        <div class="form-row">
          <label class="form-row-label">{{ t('bhyve.memory') }}</label>
          <div>
            <input type="text" v-model="form.memory" placeholder="512M" />
          </div>
        </div>
        <div class="form-row">
          <label class="form-row-label">{{ t('bhyve.diskSize') }}</label>
          <div>
            <input type="text" v-model="form.size" placeholder="20G" />
            <p class="param-desc">{{ t('bhyve.descDiskSize') }}</p>
          </div>
        </div>
      </div>
    </div>

    <div class="card" style="padding:24px; margin-top:16px;">
      <h3>{{ t('bhyve.postCreate') }}</h3>

      <!-- Install ISO -->
      <div class="form-row">
        <label class="form-row-label">{{ t('bhyve.installIso') }}</label>
        <div>
          <select v-model="form.install_iso">
            <option value="">{{ t('bhyve.noInstall') }}</option>
            <option v-for="iso in isos" :key="iso" :value="iso">{{ iso }}</option>
          </select>
          <p class="param-desc">{{ t('bhyve.descInstallIso') }}</p>
        </div>
      </div>

      <!-- Start after create -->
      <div class="form-row">
        <label class="form-row-label">{{ t('bhyve.startAfter') }}</label>
        <div>
          <label class="checkbox-label">
            <input type="checkbox" v-model="form.start_after" />
            <span class="param-desc-inline">{{ form.install_iso ? t('bhyve.descStartAfterInstall') : t('bhyve.descStartAfter') }}</span>
          </label>
        </div>
      </div>
    </div>

    <div class="form-actions-bar">
      <a href="#/bhyve/vms" class="btn btn-secondary">{{ t('common.cancel') }}</a>
      <button type="submit" :disabled="submitting">
        <span v-if="submitting" class="spinner" style="width:14px;height:14px;"></span>
        {{ submitting ? t('bhyve.creating') : t('common.confirm') }}
      </button>
    </div>
  </form>
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
.form-grid-3 {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 0 16px;
}
.form-actions-bar {
  display: flex;
  gap: 12px;
  justify-content: flex-end;
  margin-top: 16px;
}
</style>
