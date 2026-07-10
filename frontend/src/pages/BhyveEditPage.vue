<script setup>
import { ref, reactive, computed, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import BackButton from '../components/ui/BackButton.vue';
import FilePicker from '../components/ui/FilePicker.vue';
import SectionCard from '../components/ui/SectionCard.vue';
import FieldHelp from '../components/ui/FieldHelp.vue';
import WarnBanner from '../components/ui/WarnBanner.vue';

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const toast = useToast();
const alert = useAlert();
const name = route.params.name;

const loading = ref(true);
const saving = ref(false);
const error = ref('');
const config = reactive({});
const switches = ref([]);
const pickerTarget = ref('');
const newKey = ref('');
const activeTab = ref('basic');

const sections = computed(() => [
  { key: 'basic', label: t('bhyve.editBasic') },
  { key: 'disks', label: t('common.disks') },
  { key: 'networks', label: t('common.network') },
  { key: 'graphics', label: t('bhyve.editGraphics') },
  { key: 'otherDevices', label: t('bhyve.editOtherDevices') },
]);

const loaderOptions = ['bhyveload', 'grub', 'uefi', 'uefi-csm'];
const hostbridgeOptions = ['standard', 'amd', 'none'];
const comportOptions = ['com1', 'com2', 'com1 com2', 'com2 com1'];
const diskTypeOptions = ['virtio-blk', 'ahci-hd', 'ahci-cd', 'nvme', 'virtio-9p'];
const diskDevOptions = ['file', 'zvol', 'sparse-zvol', 'custom', 'iscsi'];
const networkTypeOptions = ['virtio-net', 'e1000'];
const resolutionOptions = ['1920x1200', '1920x1080', '1600x1200', '1600x900', '1280x1024', '1280x720', '1024x768', '800x600', '640x480'];
const graphicsWaitOptions = ['no', 'yes', 'auto'];
const graphicsVgaOptions = ['on', 'off', 'io'];
const diskIndexes = computed(() => indexesFor('disk'));
const networkIndexes = computed(() => indexesFor('network'));
const passthruIndexes = computed(() => numberedIndexes('passthru'));
const consoleIndexes = computed(() => numberedIndexes('virt_console'));

const fieldKeys = {
  bhyveload_loader: 'fieldBhyveloadLoader', bhyveload_args: 'fieldBhyveloadArgs',
  cpu_sockets: 'fieldCpuSockets', cpu_cores: 'fieldCpuCores', cpu_threads: 'fieldCpuThreads',
  hostbridge: 'fieldHostbridge', comports: 'fieldComports', loader_timeout: 'fieldLoaderTimeout',
  ahci_device_limit: 'fieldAhciDeviceLimit', uuid: 'fieldUuid', bhyve_options: 'fieldBhyveOptions',
  wired_memory: 'fieldWiredMemory', uefi_vars: 'fieldUefiVars', ignore_msr: 'fieldIgnoreMsr',
  utctime: 'fieldUtcTime', debug: 'fieldDebug', virt_random: 'fieldVirtRandom',
  type: 'fieldDeviceType', dev: 'fieldDiskBackend', name: 'fieldDeviceName', opts: 'fieldDeviceOptions',
  switch: 'fieldSwitch', device: 'fieldHostInterface', mac: 'fieldMacAddress', span: 'fieldSpanPort',
  graphics: 'fieldGraphicsEnabled', graphics_port: 'fieldGraphicsPort', graphics_listen: 'fieldGraphicsListen',
  graphics_res: 'fieldGraphicsResolution', graphics_wait: 'fieldGraphicsWait', graphics_vga: 'fieldGraphicsVga',
  vnc_password: 'fieldVncPassword', xhci_mouse: 'fieldXhciMouse', sound: 'fieldSoundEnabled',
  sound_play: 'fieldSoundPlayback', sound_rec: 'fieldSoundRecording',
};

const fieldHints = {
  bhyveload_loader: 'fieldBhyveloadLoaderHint', bhyveload_args: 'fieldBhyveloadArgsHint',
  cpu_sockets: 'fieldCpuTopologyHint', cpu_cores: 'fieldCpuTopologyHint', cpu_threads: 'fieldCpuTopologyHint',
  hostbridge: 'fieldHostbridgeHint', comports: 'fieldComportsHint', loader_timeout: 'fieldLoaderTimeoutHint',
  ahci_device_limit: 'fieldAhciDeviceLimitHint', bhyve_options: 'fieldBhyveOptionsHint',
  wired_memory: 'fieldWiredMemoryHint', uefi_vars: 'fieldUefiVarsHint', ignore_msr: 'fieldIgnoreMsrHint',
  utctime: 'fieldUtcTimeHint', debug: 'fieldDebugHint', virt_random: 'fieldVirtRandomHint',
  dev: 'fieldDiskBackendHint', opts: 'fieldDeviceOptionsHint', switch: 'fieldSwitchHint',
  device: 'fieldHostInterfaceHint', mac: 'fieldMacAddressHint', span: 'fieldSpanPortHint',
  graphics: 'fieldGraphicsEnabledHint', graphics_port: 'fieldGraphicsPortHint', graphics_listen: 'fieldGraphicsListenHint',
  graphics_res: 'fieldGraphicsResolutionHint', graphics_wait: 'fieldGraphicsWaitHint', graphics_vga: 'fieldGraphicsVgaHint',
  vnc_password: 'fieldVncPasswordHint', xhci_mouse: 'fieldXhciMouseHint', sound: 'fieldSoundEnabledHint',
};

function fieldLabel(key) {
  return fieldKeys[key] ? t(`bhyve.${fieldKeys[key]}`) : key;
}

function fieldHint(key) {
  return fieldHints[key] ? t(`bhyve.${fieldHints[key]}`) : '';
}

function indexesFor(prefix) {
  return [...new Set(Object.keys(config)
    .map((key) => key.match(new RegExp(`^${prefix}(\\d+)_`))?.[1])
    .filter(Boolean))]
    .sort((a, b) => Number(a) - Number(b));
}

function numberedIndexes(prefix) {
  return [...new Set(Object.keys(config)
    .map((key) => key.match(new RegExp(`^${prefix}(\\d+)$`))?.[1])
    .filter(Boolean))]
    .sort((a, b) => Number(a) - Number(b));
}

function ensure(key, value = '') {
  if (!(key in config)) config[key] = value;
}

function remove(key) {
  delete config[key];
}

function boolValue(key) {
  return !['', 'no', 'off', 'false', '0'].includes(String(config[key] || '').toLowerCase());
}

function setBool(key, enabled) {
  config[key] = enabled ? 'yes' : 'no';
}

function deviceKey(prefix, index, suffix) {
  return `${prefix}${index}_${suffix}`;
}

function addDisk() {
  const index = diskIndexes.value.length ? Math.max(...diskIndexes.value.map(Number)) + 1 : 0;
  config[deviceKey('disk', index, 'type')] = 'virtio-blk';
  config[deviceKey('disk', index, 'dev')] = 'file';
  config[deviceKey('disk', index, 'name')] = `disk${index}.img`;
  config[deviceKey('disk', index, 'opts')] = '';
}

function onDiskTypeChange(index) {
  const typeKey = deviceKey('disk', index, 'type');
  const devKey = deviceKey('disk', index, 'dev');
  const nameKey = deviceKey('disk', index, 'name');
  if (config[typeKey] === 'virtio-9p') {
    config[devKey] = 'custom';
    if (!String(config[nameKey] || '').includes('=')) config[nameKey] = `data=/vm/${name}/data`;
  }
}

function ninepShare(index) {
  return String(config[deviceKey('disk', index, 'name')] || '').split('=')[0] || 'data';
}

function ninepPath(index) {
  const value = String(config[deviceKey('disk', index, 'name')] || '');
  const separator = value.indexOf('=');
  return separator >= 0 ? value.slice(separator + 1) : '';
}

function setNinepShare(index, share) {
  config[deviceKey('disk', index, 'name')] = `${share || 'data'}=${ninepPath(index)}`;
}

function setNinepPath(index, path) {
  config[deviceKey('disk', index, 'name')] = `${ninepShare(index)}=${path}`;
}

function openNinepPicker(index) {
  pickerTarget.value = `ninep:${index}`;
}

function removeDevice(prefix, index) {
  for (const key of Object.keys(config)) {
    if (key.startsWith(`${prefix}${index}_`)) delete config[key];
  }
}

function addNetwork() {
  const index = networkIndexes.value.length ? Math.max(...networkIndexes.value.map(Number)) + 1 : 0;
  config[deviceKey('network', index, 'type')] = 'virtio-net';
  config[deviceKey('network', index, 'switch')] = switches.value[0]?.name || '';
  config[deviceKey('network', index, 'span')] = 'no';
}

function addPassthru() {
  const index = passthruIndexes.value.length ? Math.max(...passthruIndexes.value.map(Number)) + 1 : 0;
  config[`passthru${index}`] = '';
}

function addConsole() {
  const index = consoleIndexes.value.length ? Math.max(...consoleIndexes.value.map(Number)) + 1 : 0;
  config[`virt_console${index}`] = 'yes';
}

function addAdvanced() {
  const key = newKey.value.trim();
  if (!key || key in config) return;
  config[key] = newValue.value;
  newKey.value = '';
  newValue.value = '';
}

function isAdvanced(key) {
  const known = new Set([
    'loader', 'bhyveload_loader', 'bhyveload_args', 'uefi_vars',
    'cpu', 'memory', 'wired_memory',
    'ignore_msr', 'bhyve_options', 'utctime', 'debug',
    'uuid', 'start_slot', 'install_slot', 'virt_random',
    'graphics', 'graphics_port', 'graphics_listen', 'graphics_res', 'graphics_wait',
    'graphics_vga', 'vnc_password', 'xhci_mouse', 'sound', 'sound_play', 'sound_rec',
  ]);
  if (/^passthru\d+$/.test(key) || /^virt_console\d+$/.test(key)) return false;
  return !known.has(key) && !/^disk\d+_/.test(key) && !/^network\d+_/.test(key);
}

const advancedKeys = computed(() => Object.keys(config).filter(isAdvanced).sort());

function openPicker(target) {
  pickerTarget.value = target;
}

function onPickerSelect(path) {
  if (pickerTarget.value.startsWith('ninep:')) {
    setNinepPath(pickerTarget.value.slice(6), path);
  } else {
    config[pickerTarget.value] = path;
  }
  pickerTarget.value = '';
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    const [detail, switchList] = await Promise.all([
      api.get(`/api/bhyve/vms/${encodeURIComponent(name)}`),
      api.get('/api/bhyve/switches'),
    ]);
    for (const key of Object.keys(config)) delete config[key];
    Object.assign(config, detail.config || {});
    switches.value = switchList;
    ensure('loader', 'uefi');
    ensure('cpu', '1');
    ensure('memory', '512M');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function save() {
  if (!Object.keys(config).length) {
    await alert(t('common.operationFailed'), t('bhyve.configInvalid'));
    return;
  }
  saving.value = true;
  try {
    await api.put(`/api/bhyve/vms/${encodeURIComponent(name)}`, { config: { ...config } });
    toast.toast(t('bhyve.configSaved'));
    router.push(`/bhyve/detail/${name}`);
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton :href="`#/bhyve/detail/${name}`" />
      <h1>{{ t('bhyve.configEditTitle', { name }) }}</h1>
    </div>
  </div>

  <div v-if="error" class="empty">{{ t('common.loadFailed', { msg: error }) }}</div>
  <div v-else-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>

  <form v-else @submit.prevent="save">
    <WarnBanner :message="t('bhyve.configEditHint')" />
    <SectionCard v-model="activeTab" :tabs="sections">
      <template #default="{ active }">
        <template v-if="active === 'basic'">
          <div class="form-row"><label class="form-row-label">{{ t('bhyve.loader') }}</label><select v-model="config.loader"><option v-for="value in loaderOptions" :key="value" :value="value">{{ value }}</option></select></div>
          <div v-if="config.loader === 'bhyveload'" class="form-row"><label class="form-row-label">{{ fieldLabel('bhyveload_loader') }}<FieldHelp :text="fieldHint('bhyveload_loader')" /></label><div class="input-with-btn"><input v-model="config.bhyveload_loader" placeholder="/boot/userboot.so" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('bhyveload_loader')"><i class="fa-solid fa-folder-open"></i></button></div></div>
          <div v-if="config.loader === 'bhyveload'" class="form-row"><label class="form-row-label">{{ fieldLabel('bhyveload_args') }}<FieldHelp :text="fieldHint('bhyveload_args')" /></label><input v-model="config.bhyveload_args" /></div>
          <div class="form-row"><label class="form-row-label">{{ t('bhyve.cpuCores') }}</label><input v-model.number="config.cpu" type="number" min="1" /></div>
          <div class="form-row"><label class="form-row-label">{{ t('bhyve.memory') }}</label><input v-model="config.memory" placeholder="1024M" /></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('uuid') }}</label><input v-model="config.uuid" /></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('bhyve_options') }}<FieldHelp :text="fieldHint('bhyve_options')" /></label><input v-model="config.bhyve_options" /></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('wired_memory') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('wired_memory')" @change="setBool('wired_memory', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('wired_memory') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('uefi_vars') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('uefi_vars')" @change="setBool('uefi_vars', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('uefi_vars') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('ignore_msr') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('ignore_msr')" @change="setBool('ignore_msr', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('ignore_msr') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('utctime') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('utctime')" @change="setBool('utctime', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('utctime') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('debug') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('debug')" @change="setBool('debug', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('debug') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('virt_random') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('virt_random')" @change="setBool('virt_random', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('virt_random') }}</span></label></div></div>
        </template>

        <template v-if="active === 'disks'">
          <div v-if="!diskIndexes.length" class="empty">{{ t('bhyve.noDisks') }}</div>
          <div v-for="index in diskIndexes" :key="index" class="card" style="margin-bottom:12px;">
            <div class="flex" style="margin-bottom:12px;"><h3 style="margin:0;">{{ t('common.disks') }} {{ index }}</h3><button type="button" class="btn-danger btn-sm" style="margin-left:auto;" @click="removeDevice('disk', index)"><i class="fa-solid fa-trash"></i></button></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('type') }}</label><select v-model="config[deviceKey('disk', index, 'type')]" @change="onDiskTypeChange(index)"><option v-for="value in diskTypeOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <template v-if="config[deviceKey('disk', index, 'type')] === 'virtio-9p'">
              <div class="form-row"><label class="form-row-label">{{ t('bhyve.ninepShare') }}</label><input :value="ninepShare(index)" @input="setNinepShare(index, $event.target.value)" placeholder="data" /></div>
              <div class="form-row"><label class="form-row-label">{{ t('bhyve.ninepPath') }}</label><div class="input-with-btn"><input :value="ninepPath(index)" @input="setNinepPath(index, $event.target.value)" placeholder="/vm/guest/data" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openNinepPicker(index)"><i class="fa-solid fa-folder-open"></i></button></div></div>
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('opts') }}<FieldHelp :text="fieldHint('opts')" /></label><input v-model="config[deviceKey('disk', index, 'opts')]" placeholder="ro" /></div>
            </template>
            <template v-else>
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('dev') }}<FieldHelp :text="fieldHint('dev')" /></label><select v-model="config[deviceKey('disk', index, 'dev')]"><option v-for="value in diskDevOptions" :key="value" :value="value">{{ value }}</option></select></div>
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('name') }}</label><div v-if="config[deviceKey('disk', index, 'dev')] === 'custom'" class="input-with-btn"><input v-model="config[deviceKey('disk', index, 'name')]" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker(deviceKey('disk', index, 'name'))"><i class="fa-solid fa-folder-open"></i></button></div><input v-else v-model="config[deviceKey('disk', index, 'name')]" /></div>
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('opts') }}<FieldHelp :text="fieldHint('opts')" /></label><input v-model="config[deviceKey('disk', index, 'opts')]" placeholder="direct,nocache,ro" /></div>
            </template>
          </div>
          <button type="button" class="btn-secondary" @click="addDisk"><i class="fa-solid fa-plus"></i> {{ t('bhyve.addDisk') }}</button>
        </template>

        <template v-if="active === 'networks'">
          <div v-if="!networkIndexes.length" class="empty">{{ t('bhyve.noNetworks') }}</div>
          <div v-for="index in networkIndexes" :key="index" class="card" style="margin-bottom:12px;">
            <div class="flex" style="margin-bottom:12px;"><h3 style="margin:0;">{{ t('common.network') }} {{ index }}</h3><button type="button" class="btn-danger btn-sm" style="margin-left:auto;" @click="removeDevice('network', index)"><i class="fa-solid fa-trash"></i></button></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('type') }}</label><select v-model="config[deviceKey('network', index, 'type')]"><option v-for="value in networkTypeOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('switch') }}<FieldHelp :text="fieldHint('switch')" /></label><select v-model="config[deviceKey('network', index, 'switch')]"><option value=""></option><option v-for="sw in switches" :key="sw.name" :value="sw.name">{{ sw.name }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('device') }}<FieldHelp :text="fieldHint('device')" /></label><input v-model="config[deviceKey('network', index, 'device')]" placeholder="tap0" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('name') }}</label><input v-model="config[deviceKey('network', index, 'name')]" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('mac') }}<FieldHelp :text="fieldHint('mac')" /></label><input v-model="config[deviceKey('network', index, 'mac')]" placeholder="58:9c:fc:00:00:01" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('span') }}<FieldHelp :text="fieldHint('span')" /></label><input type="checkbox" :checked="boolValue(deviceKey('network', index, 'span'))" @change="setBool(deviceKey('network', index, 'span'), $event.target.checked)" /></div>
          </div>
          <button type="button" class="btn-secondary" @click="addNetwork"><i class="fa-solid fa-plus"></i> {{ t('bhyve.addNetwork') }}</button>
        </template>

        <template v-if="active === 'graphics'">
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('graphics')" @change="setBool('graphics', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('graphics') }}</span></label></div></div>
          <template v-if="boolValue('graphics')">
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_port') }}<FieldHelp :text="fieldHint('graphics_port')" /></label><input v-model.number="config.graphics_port" type="number" min="1" max="65535" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_listen') }}<FieldHelp :text="fieldHint('graphics_listen')" /></label><input v-model="config.graphics_listen" placeholder="0.0.0.0" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_res') }}<FieldHelp :text="fieldHint('graphics_res')" /></label><select v-model="config.graphics_res"><option value=""></option><option v-for="value in resolutionOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_wait') }}<FieldHelp :text="fieldHint('graphics_wait')" /></label><select v-model="config.graphics_wait"><option v-for="value in graphicsWaitOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_vga') }}<FieldHelp :text="fieldHint('graphics_vga')" /></label><select v-model="config.graphics_vga"><option v-for="value in graphicsVgaOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('vnc_password') }}<FieldHelp :text="fieldHint('vnc_password')" /></label><input v-model="config.vnc_password" type="password" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('xhci_mouse') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('xhci_mouse')" @change="setBool('xhci_mouse', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('xhci_mouse') }}</span></label></div></div>
          </template>
        </template>

        <template v-if="active === 'otherDevices'">
          <div class="card" style="margin-bottom:12px;">
            <h3>{{ t('bhyve.editPciPassthru') }}</h3>
            <p class="param-desc">{{ t('bhyve.editPciPassthruHint') }}</p>
            <div v-for="index in passthruIndexes" :key="index" class="form-row">
              <label class="form-row-label">{{ t('bhyve.fieldPciDevice') }} {{ index }}</label>
              <div class="input-with-btn"><input v-model="config[`passthru${index}`]" placeholder="3/0/0 or 6/0/0=2:0" /><button type="button" class="btn-danger btn-sm fp-trigger" @click="remove(`passthru${index}`)"><i class="fa-solid fa-trash"></i></button></div>
            </div>
            <button type="button" class="btn-secondary" @click="addPassthru"><i class="fa-solid fa-plus"></i> {{ t('bhyve.addPciPassthru') }}</button>
          </div>
          <div class="card" style="margin-bottom:12px;">
            <h3>{{ t('bhyve.editVirtConsole') }}</h3>
            <p class="param-desc">{{ t('bhyve.editVirtConsoleHint') }}</p>
            <div v-for="index in consoleIndexes" :key="index" class="form-row">
              <label class="form-row-label">{{ t('bhyve.fieldVirtConsole') }} {{ index }}</label>
              <div class="input-with-btn"><input v-model="config[`virt_console${index}`]" placeholder="yes or org.freenas.bhyve-agent" /><button type="button" class="btn-danger btn-sm fp-trigger" @click="remove(`virt_console${index}`)"><i class="fa-solid fa-trash"></i></button></div>
            </div>
            <button type="button" class="btn-secondary" @click="addConsole"><i class="fa-solid fa-plus"></i> {{ t('bhyve.addVirtConsole') }}</button>
          </div>
          <div class="card">
            <h3>{{ t('bhyve.editSound') }}</h3>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('sound') }}<FieldHelp :text="fieldHint('sound')" /></label><input type="checkbox" :checked="boolValue('sound')" @change="setBool('sound', $event.target.checked)" /></div>
            <template v-if="boolValue('sound')">
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('sound_play') }}</label><div class="input-with-btn"><input v-model="config.sound_play" placeholder="/dev/dsp0" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('sound_play')"><i class="fa-solid fa-folder-open"></i></button></div></div>
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('sound_rec') }}</label><div class="input-with-btn"><input v-model="config.sound_rec" placeholder="/dev/dsp0" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('sound_rec')"><i class="fa-solid fa-folder-open"></i></button></div></div>
            </template>
          </div>
        </template>

      </template>
    </SectionCard>

    <div class="card" style="margin-top:16px;">
      <h3>{{ t('bhyve.editAdvanced') }}</h3>
      <div v-for="key in advancedKeys" :key="key" class="form-row">
        <label class="form-row-label">{{ t('bhyve.fieldAdvancedKey', { key }) }}</label>
        <div class="input-with-btn"><input v-model="config[key]" /><button type="button" class="btn-danger btn-sm fp-trigger" @click="remove(key)"><i class="fa-solid fa-trash"></i></button></div>
      </div>
      <div class="form-row"><label class="form-row-label">{{ t('bhyve.addParameter') }}</label><div class="input-with-btn"><input v-model="newKey" :placeholder="t('bhyve.configKeyPlaceholder')" /><input v-model="newValue" :placeholder="t('bhyve.configValuePlaceholder')" @keyup.enter="addAdvanced" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="addAdvanced"><i class="fa-solid fa-plus"></i></button></div></div>
    </div>

      <div class="form-actions-bar">
        <a :href="`#/bhyve/detail/${name}`" class="btn btn-secondary">{{ t('common.cancel') }}</a>
        <button type="submit" :disabled="saving">{{ t('common.save') }}</button>
      </div>
  </form>

  <FilePicker v-if="pickerTarget" :mode="pickerTarget.startsWith('ninep:') ? 'dir' : 'file'" :initial-path="pickerTarget.startsWith('ninep:') ? ninepPath(pickerTarget.slice(6)) || '/' : config[pickerTarget] || '/'" @select="onPickerSelect" @close="pickerTarget = ''" />
</template>
