<script setup>
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import BackButton from '../components/ui/BackButton.vue';
import SectionCard from '../components/ui/SectionCard.vue';
import FieldHelp from '../components/ui/FieldHelp.vue';
import ComboBox from '../components/ui/ComboBox.vue';
import FilePicker from '../components/ui/FilePicker.vue';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const alert = useAlert();

const templates = ref([]);
const datastores = ref([]);
const images = ref([]);
const isos = ref([]);
const submitting = ref(false);
const pickerTarget = ref('');

const sections = computed(() => [
  { key: 'basic', label: t('bhyve.editBasic') },
  { key: 'install', label: t('bhyve.installMethod') },
]);

const form = ref({
  name: '',
  template: 'default',
  datastore: '',
  size: '20G',
  cpu: 1,
  memory: '512M',
  install_method: 'none',
  image: '',
  iso: '',
});

async function loadOptions() {
  try {
    const [tmpls, ds, imgFiles, isoList] = await Promise.all([
      api.get('/api/bhyve/templates'),
      api.get('/api/bhyve/datastores'),
      api.get('/api/bhyve/img-files'),
      api.get('/api/bhyve/isos'),
    ]);
    templates.value = tmpls;
    datastores.value = ds;
    images.value = imgFiles;
    isos.value = isoList;
    if (ds.length) form.value.datastore = ds[0].name;
  } catch {
    // ignore — selectors will be empty
  }
}

function openPicker(target) {
  pickerTarget.value = target;
}

function onPickerSelect(path) {
  if (pickerTarget.value === 'image') {
    form.value.image = path;
  } else if (pickerTarget.value === 'iso') {
    form.value.iso = path;
  }
  pickerTarget.value = '';
}

async function onSubmit() {
  if (!form.value.name) {
    await alert(t('common.operationFailed'), t('bhyve.errNameRequired'));
    return;
  }
  if (!form.value.template) {
    await alert(t('common.operationFailed'), t('bhyve.errTemplateRequired'));
    return;
  }

  const installMethod = form.value.install_method;

  if (installMethod === 'image' && !form.value.image) {
    await alert(t('common.operationFailed'), t('bhyve.errImageRequired'));
    return;
  }
  if (installMethod === 'iso' && !form.value.iso) {
    await alert(t('common.operationFailed'), t('bhyve.errIsoRequired'));
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

  if (installMethod === 'image') {
    body.image = form.value.image;
  }

  submitting.value = true;
  try {
    await api.post('/api/bhyve/vms', body);
    if (installMethod === 'iso') {
      try {
        await api.post(`/api/bhyve/vms/${encodeURIComponent(form.value.name)}/install`, {
          iso: form.value.iso,
        });
      } catch {
        // install failure is non-fatal — VM was created
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
    <SectionCard :tabs="sections" expand>
      <template #default="{ active }">

        <!-- ── basic ── -->
        <template v-if="active === 'basic'">
          <div class="form-row">
            <label class="form-row-label">{{ t('bhyve.vmName') }} <span style="color:var(--danger)">*</span><FieldHelp :text="t('bhyve.descVmName')" /></label>
            <div>
              <input type="text" v-model="form.name" required placeholder="my-vm"
                pattern="[a-z0-9][a-z0-9._-]*[a-z0-9]" />
            </div>
          </div>

          <div class="form-row">
            <label class="form-row-label">{{ t('bhyve.template') }} <span style="color:var(--danger)">*</span><FieldHelp :text="t('bhyve.descTemplate')" /></label>
            <div>
              <select v-model="form.template" required>
                <option v-for="tp in templates" :key="tp" :value="tp">{{ tp }}</option>
              </select>
            </div>
          </div>

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

          <div class="form-grid-3">
            <div class="form-row">
              <label class="form-row-label">{{ t('bhyve.cpuCores') }}</label>
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
              <label class="form-row-label">{{ t('bhyve.diskSize') }}<FieldHelp :text="t('bhyve.descDiskSize')" /></label>
              <div>
                <input type="text" v-model="form.size" placeholder="20G" />
              </div>
            </div>
          </div>
        </template>

        <!-- ── install ── -->
        <template v-if="active === 'install'">
          <div class="form-row">
            <label class="form-row-label">{{ t('bhyve.installMethod') }}</label>
            <div class="radio-group-inline">
              <label class="radio-label">
                <input type="radio" v-model="form.install_method" value="none" />
                <span>{{ t('bhyve.installNone') }}</span>
              </label>
              <label class="radio-label">
                <input type="radio" v-model="form.install_method" value="image" />
                <span>{{ t('bhyve.installImage') }}</span>
              </label>
              <label class="radio-label">
                <input type="radio" v-model="form.install_method" value="iso" />
                <span>{{ t('bhyve.installIso') }}</span>
              </label>
            </div>
          </div>

          <div v-if="form.install_method === 'image'" class="form-row">
            <label class="form-row-label">{{ t('bhyve.selectImage') }}<FieldHelp :text="t('bhyve.descSelectImage')" /></label>
            <div>
              <div class="input-with-btn">
                <ComboBox v-model="form.image" :options="images.map(i => ({ value: i.name, label: i.name }))" placeholder="/path/to/disk.img" />
                <button type="button" class="btn-secondary btn-sm" @click="openPicker('image')"><i class="fa-solid fa-folder-open"></i></button>
              </div>
            </div>
          </div>

          <div v-if="form.install_method === 'iso'" class="form-row">
            <label class="form-row-label">ISO<FieldHelp :text="t('bhyve.descSelectIso')" /></label>
            <div>
              <div class="input-with-btn">
                <ComboBox v-model="form.iso" :options="isos.map(i => ({ value: i.name, label: i.name }))" placeholder="/path/to/install.iso" />
                <button type="button" class="btn-secondary btn-sm" @click="openPicker('iso')"><i class="fa-solid fa-folder-open"></i></button>
              </div>
            </div>
          </div>
        </template>

      </template>
    </SectionCard>

    <FilePicker v-if="pickerTarget" mode="file"
      :accept="pickerTarget === 'iso' ? ['.iso'] : []"
      :initial-path="form[pickerTarget] || '/'"
      @select="onPickerSelect" @close="pickerTarget = ''" />

    <div class="form-actions-bar">
      <a href="#/bhyve/vms" class="btn btn-secondary">{{ t('common.cancel') }}</a>
      <button type="submit" :disabled="submitting">
        <span v-if="submitting" class="spinner" style="width:14px;height:14px;"></span>
        {{ submitting ? t('bhyve.creating') : t('common.confirm') }}
      </button>
    </div>
  </form>
</template>
