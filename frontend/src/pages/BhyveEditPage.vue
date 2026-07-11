<script setup>
import { ref, reactive, computed, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';
import { fmtBytesStr } from '../lib/format.js';
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
const confirm = useConfirm();
const formModal = useFormModal();
const name = route.params.name;

const loading = ref(true);
const saving = ref(false);
const error = ref('');
const config = reactive({});
const disks = ref([]);
const networks = ref([]);
const switches = ref([]);
const diskResources = ref({ files: [], zvols: [] });
const pickerTarget = ref('');
const newKey = ref('');
const newValue = ref('');
const activeTab = ref('basic');

/* ── disk edit modal state ── */
const diskModal = reactive({
  visible: false,
  mode: 'edit',
  index: null,
  origIndex: null,
  type: 'virtio-blk',
  dev: 'file',
  name: '',
  opts: '',
  ninepShare: 'data',
  ninepPath: '',
  pickerField: '',
});

/* ── network edit modal state ── */
const netModal = reactive({
  visible: false,
  mode: 'edit',
  index: null,
  origIndex: null,
  type: 'virtio-net',
  switch: '',
  mac: '',
});

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
const diskTypeLabels = { 'virtio-blk': 'VirtIO', 'ahci-hd': 'bhyve.diskTypeAhciHd', 'ahci-cd': 'bhyve.diskTypeAhciCd', 'nvme': 'NVMe', 'virtio-9p': 'bhyve.diskTypeVirtio9p' };
function diskTypeLabel(value) {
  const label = diskTypeLabels[value];
  if (!label) return value;
  return label.startsWith('bhyve.') ? t(label) : label;
}
const diskDevOptions = ['file', 'zvol', 'custom', 'iscsi'];
const diskDevLabels = { 'file': 'bhyve.diskDevFile', 'zvol': 'ZVol', 'custom': 'bhyve.diskDevCustom', 'iscsi': 'iSCSI' };
function diskDevLabel(value) {
  const label = diskDevLabels[value];
  if (!label) return value;
  return label.startsWith('bhyve.') ? t(label) : label;
}
const networkTypeOptions = ['virtio-net', 'e1000'];
const networkTypeLabels = { 'virtio-net': 'VirtIO', 'e1000': 'E1000' };
function networkTypeLabel(value) {
  return networkTypeLabels[value] || value;
}
const resolutionOptions = ['1920x1200', '1920x1080', '1600x1200', '1600x900', '1280x1024', '1280x720', '1024x768', '800x600', '640x480'];
const graphicsWaitOptions = ['no', 'yes', 'auto'];
const graphicsVgaOptions = ['on', 'off', 'io'];

const passthruIndexes = computed(() => numberedIndexes('passthru'));
const consoleIndexes = computed(() => numberedIndexes('virt_console'));

const basicKeyList = ['loader', 'uefi_vars', 'cpu', 'memory', 'wired_memory', 'ignore_msr', 'bhyve_options', 'utctime', 'debug', 'uuid', 'start_slot', 'install_slot', 'virt_random'];
const graphicsKeyList = ['graphics', 'graphics_port', 'graphics_listen', 'graphics_res', 'graphics_wait', 'graphics_vga', 'vnc_password', 'xhci_mouse'];
const soundKeyList = ['sound', 'sound_play', 'sound_rec'];

function isOtherDeviceKey(key) {
  if (/^passthru\d+$/.test(key) || /^virt_console\d+$/.test(key)) return true;
  return soundKeyList.includes(key);
}

const fieldKeys = {
  bhyveload_loader: 'fieldBhyveloadLoader', bhyveload_args: 'fieldBhyveloadArgs',
  cpu_sockets: 'fieldCpuSockets', cpu_cores: 'fieldCpuCores', cpu_threads: 'fieldCpuThreads',
  hostbridge: 'fieldHostbridge', comports: 'fieldComports', loader_timeout: 'fieldLoaderTimeout',
  ahci_device_limit: 'fieldAhciDeviceLimit', uuid: 'fieldUuid', bhyve_options: 'fieldBhyveOptions',
  wired_memory: 'fieldWiredMemory', uefi_vars: 'fieldUefiVars', ignore_msr: 'fieldIgnoreMsr',
  utctime: 'fieldUtcTime', debug: 'fieldDebug', virt_random: 'fieldVirtRandom',
  type: 'fieldDeviceType', dev: 'fieldDiskBackend', name: 'fieldDeviceName', opts: 'fieldDeviceOptions',
  switch: 'fieldSwitch', device: 'fieldHostInterface', mac: 'fieldMacAddress',
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
  device: 'fieldHostInterfaceHint', mac: 'fieldMacAddressHint',
  graphics: 'fieldGraphicsEnabledHint', graphics_port: 'fieldGraphicsPortHint', graphics_listen: 'fieldGraphicsListenHint',
  graphics_res: 'fieldGraphicsResolutionHint', graphics_wait: 'fieldGraphicsWaitHint', graphics_vga: 'fieldGraphicsVgaHint',
  vnc_password: 'fieldVncPasswordHint', xhci_mouse: 'fieldXhciMouseHint', sound: 'fieldSoundEnabledHint',
};

function fieldLabel(key) {
  return fieldKeys[key] ? t(`bhyve.${fieldKeys[key]}`) : key;
}

function diskNameLabel(dev) {
  switch (dev) {
    case 'zvol': return t('bhyve.fieldDiskZvolName');
    case 'custom': return t('bhyve.fieldDiskCustomPath');
    case 'iscsi': return t('bhyve.fieldDiskIscsiTarget');
    default: return t('bhyve.fieldDiskFileName');
  }
}

function diskNameHint(dev) {
  switch (dev) {
    case 'zvol': return t('bhyve.fieldDiskZvolNameHint');
    case 'custom': return t('bhyve.fieldDiskCustomPathHint');
    case 'iscsi': return t('bhyve.fieldDiskIscsiTargetHint');
    default: return t('bhyve.fieldDiskFileNameHint');
  }
}

function diskNamePlaceholder(dev) {
  switch (dev) {
    case 'zvol': return 'disk1';
    case 'custom': return '/dev/zvol/zroot/disks/disk1';
    case 'iscsi': return '1/0';
    default: return 'disk0.img';
  }
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

function removeDevice(prefix, index) {
  for (const key of Object.keys(config)) {
    if (key.startsWith(`${prefix}${index}_`)) delete config[key];
  }
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
    'loader', 'uefi_vars',
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
  if (pickerTarget.value === 'modal:name') {
    diskModal.name = path;
  } else if (pickerTarget.value === 'modal:ninepPath') {
    diskModal.ninepPath = path;
  } else {
    config[pickerTarget.value] = path;
  }
  pickerTarget.value = '';
}

/* ── disk helpers ── */

function diskDisplayName(disk) {
  if (disk.type === 'virtio-9p') return `${disk.ninepShare}=${disk.ninepPath}`;
  return disk.name;
}

function usedDiskNames(devType, excludeIndex) {
  return new Set(disks.value
    .filter((d) => d.index !== excludeIndex && d.dev === devType)
    .map((d) => d.name));
}

function modalAvailableFiles() {
  const used = usedDiskNames('file', diskModal.index);
  return diskResources.value.files.filter((f) => f === diskModal.name || !used.has(f));
}

function modalAvailableZvols() {
  const used = usedDiskNames('zvol', diskModal.index);
  return diskResources.value.zvols.filter((z) => z === diskModal.name || !used.has(z));
}

/* ── disk modal operations ── */

function openEditDisk(disk) {
  diskModal.mode = 'edit';
  diskModal.index = disk.index;
  diskModal.origIndex = disk.index;
  diskModal.type = disk.type;
  diskModal.dev = disk.dev;
  diskModal.opts = disk.opts || '';
  diskModal.name = disk.name || '';
  diskModal.ninepShare = disk.ninepShare || 'data';
  diskModal.ninepPath = disk.ninepPath || '';
  diskModal.visible = true;
}

function openImportDisk() {
  diskModal.mode = 'import';
  diskModal.index = disks.value.length ? Math.max(...disks.value.map((d) => d.index)) + 1 : 0;
  diskModal.origIndex = null;
  diskModal.type = 'virtio-blk';
  diskModal.dev = 'file';
  diskModal.name = '';
  diskModal.opts = '';
  diskModal.ninepShare = 'data';
  diskModal.ninepPath = '';
  diskModal.visible = true;
}

function onModalTypeChange() {
  if (diskModal.type === 'virtio-9p') {
    diskModal.dev = 'custom';
  } else if (diskModal.dev === 'custom') {
    diskModal.dev = 'file';
  }
}

async function saveDiskModal() {
  const newIndex = Number(diskModal.index);
  if (isNaN(newIndex) || newIndex < 0) {
    await alert(t('common.operationFailed'), t('bhyve.invalidDiskIndex'));
    return;
  }
  const payload = {};
  payload[`disk${newIndex}_type`] = diskModal.type;
  if (diskModal.type === 'virtio-9p') {
    payload[`disk${newIndex}_name`] = `${diskModal.ninepShare || 'data'}=${diskModal.ninepPath}`;
    payload[`disk${newIndex}_dev`] = 'custom';
  } else {
    if (diskModal.dev !== 'file') payload[`disk${newIndex}_dev`] = diskModal.dev;
    payload[`disk${newIndex}_name`] = diskModal.name;
  }
  if (diskModal.opts.trim()) payload[`disk${newIndex}_opts`] = diskModal.opts.trim();

  const deletePayload = {};
  if (diskModal.mode === 'edit' && diskModal.origIndex !== newIndex) {
    for (const key of [`disk${diskModal.origIndex}_type`, `disk${diskModal.origIndex}_name`, `disk${diskModal.origIndex}_dev`, `disk${diskModal.origIndex}_opts`]) {
      deletePayload[key] = '';
    }
  }

  saving.value = true;
  try {
    const fullPayload = { ...deletePayload, ...payload };
    await api.put(`/api/bhyve/vms/${encodeURIComponent(name)}`, { config: fullPayload });
    toast.toast(t('common.saved'));
    diskModal.visible = false;
    await reloadDisks();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    saving.value = false;
  }
}

async function confirmDeleteDisk(index) {
  const ok = await confirm(t('bhyve.removeDiskTitle'), t('bhyve.removeDiskMessage'));
  if (!ok) return;
  try {
    await api.del(`/api/bhyve/vms/${encodeURIComponent(name)}/disks/${index}`);
    toast.toast(t('common.saved'));
    await reloadDisks();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

async function createDisk() {
  const result = await formModal(t('bhyve.createDiskTitle'), [
    {
      key: 'disk_type',
      label: t('bhyve.createDiskType'),
      type: 'select',
      required: true,
      value: 'zvol',
      options: [
        { value: 'zvol', label: t('bhyve.createDiskTypeZvol') },
        { value: 'sparse-zvol', label: t('bhyve.createDiskTypeSparseZvol') },
        { value: 'file', label: t('bhyve.createDiskTypeFile') },
      ],
    },
    { key: 'size', label: t('bhyve.createDiskSize'), value: '20G', placeholder: '20G', required: true },
  ], t('bhyve.createDiskConfirm'));
  if (!result) return;
  try {
    await api.post(`/api/bhyve/vms/${encodeURIComponent(name)}/disks`, result);
    toast.toast(t('common.saved'));
    await reloadDisks();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

/* ── network helpers ── */

function openEditNetwork(net) {
  netModal.mode = 'edit';
  netModal.index = net.index;
  netModal.origIndex = net.index;
  netModal.type = net.type;
  netModal.switch = net.switch || '';
  netModal.mac = net.mac || '';
  netModal.visible = true;
}

function openAddNetwork() {
  netModal.mode = 'add';
  netModal.index = networks.value.length ? Math.max(...networks.value.map((n) => n.index)) + 1 : 0;
  netModal.origIndex = null;
  netModal.type = 'virtio-net';
  netModal.switch = switches.value[0]?.name || '';
  netModal.mac = '';
  netModal.visible = true;
}

async function saveNetModal() {
  const newIndex = Number(netModal.index);
  if (isNaN(newIndex) || newIndex < 0) {
    await alert(t('common.operationFailed'), t('bhyve.invalidDiskIndex'));
    return;
  }
  const payload = {};
  payload[`network${newIndex}_type`] = netModal.type;
  if (netModal.switch) payload[`network${newIndex}_switch`] = netModal.switch;
  if (netModal.mac.trim()) payload[`network${newIndex}_mac`] = netModal.mac.trim();

  const deletePayload = {};
  if (netModal.mode === 'edit' && netModal.origIndex !== newIndex) {
    for (const key of [`network${netModal.origIndex}_type`, `network${netModal.origIndex}_switch`, `network${netModal.origIndex}_mac`]) {
      deletePayload[key] = '';
    }
  }

  saving.value = true;
  try {
    const fullPayload = { ...deletePayload, ...payload };
    await api.put(`/api/bhyve/vms/${encodeURIComponent(name)}`, { config: fullPayload });
    toast.toast(t('common.saved'));
    netModal.visible = false;
    await reloadNetworks();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    saving.value = false;
  }
}

async function confirmDeleteNetwork(index) {
  const ok = await confirm(t('bhyve.removeNetworkTitle'), t('bhyve.removeNetworkMessage'));
  if (!ok) return;
  try {
    await api.del(`/api/bhyve/vms/${encodeURIComponent(name)}/networks/${index}`);
    toast.toast(t('common.saved'));
    await reloadNetworks();
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
}

/* ── data loading ── */

function parseDisks(cfg, infoDisks) {
  const byNum = {};
  if (Array.isArray(infoDisks)) {
    for (const d of infoDisks) byNum[d.number] = d;
  }
  const indexes = [...new Set(Object.keys(cfg)
    .map((k) => k.match(/^disk(\d+)_/)?.[1])
    .filter(Boolean))]
    .sort((a, b) => Number(a) - Number(b));
  disks.value = indexes.map((i) => {
    const idx = Number(i);
    const info = byNum[idx] || {};
    const type = cfg[`disk${idx}_type`] || 'virtio-blk';
    const rawDev = cfg[`disk${idx}_dev`] || 'file';
    const dev = rawDev === 'sparse-zvol' ? 'zvol' : rawDev;
    const rawName = cfg[`disk${idx}_name`] || '';
    const opts = cfg[`disk${idx}_opts`] || '';
    const size = info.bytes_size ? fmtBytesStr(info.bytes_size) : '';
    const used = info.bytes_used ? fmtBytesStr(info.bytes_used) : '';
    if (type === 'virtio-9p') {
      const sep = rawName.indexOf('=');
      return {
        index: idx, type, dev: 'custom', name: rawName,
        ninepShare: sep >= 0 ? rawName.slice(0, sep) : rawName || 'data',
        ninepPath: sep >= 0 ? rawName.slice(sep + 1) : '',
        opts, size, used,
      };
    }
    return { index: idx, type, dev, name: rawName, opts, size, used };
  });
}

async function reloadDisks() {
  try {
    const [detail, resources] = await Promise.all([
      api.get(`/api/bhyve/vms/${encodeURIComponent(name)}`),
      api.get(`/api/bhyve/vms/${encodeURIComponent(name)}/disk-resources`),
    ]);
    parseDisks(detail.config || {}, detail.disks);
    diskResources.value = resources;
  } catch { /* silent */ }
}

function parseNetworks(cfg) {
  const indexes = [...new Set(Object.keys(cfg)
    .map((k) => k.match(/^network(\d+)_/)?.[1])
    .filter(Boolean))]
    .sort((a, b) => Number(a) - Number(b));
  networks.value = indexes.map((i) => {
    const idx = Number(i);
    return {
      index: idx,
      type: cfg[`network${idx}_type`] || 'virtio-net',
      switch: cfg[`network${idx}_switch`] || '',
      mac: cfg[`network${idx}_mac`] || '',
    };
  });
}

async function reloadNetworks() {
  try {
    const detail = await api.get(`/api/bhyve/vms/${encodeURIComponent(name)}`);
    parseNetworks(detail.config || {});
  } catch { /* silent */ }
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    const [detail, switchList, resources] = await Promise.all([
      api.get(`/api/bhyve/vms/${encodeURIComponent(name)}`),
      api.get('/api/bhyve/switches'),
      api.get(`/api/bhyve/vms/${encodeURIComponent(name)}/disk-resources`),
    ]);
    const fullConfig = detail.config || {};
    /* extract disk and network keys into separate arrays */
    parseDisks(fullConfig, detail.disks);
    parseNetworks(fullConfig);
    /* keep only non-disk, non-network keys in config */
    for (const key of Object.keys(config)) delete config[key];
    for (const [key, value] of Object.entries(fullConfig)) {
      if (!/^disk\d+_/.test(key) && !/^network\d+_/.test(key)) config[key] = value;
    }
    switches.value = switchList;
    diskResources.value = resources;
    ensure('loader', 'uefi');
    ensure('cpu', '1');
    ensure('memory', '512M');
    ensure('graphics_vga', 'off');
  } catch (e) {
    error.value = e.message || '';
  } finally {
    loading.value = false;
  }
}

async function saveBasicConfig() {
  saving.value = true;
  try {
    const configPayload = {};
    const advancePayload = {};
    for (const [key, value] of Object.entries(config)) {
      if (basicKeyList.includes(key)) configPayload[key] = String(value ?? '');
      else if (isAdvanced(key)) advancePayload[key] = String(value ?? '');
    }
    await api.put(`/api/bhyve/vms/${encodeURIComponent(name)}`, { config: configPayload, advance: advancePayload });
    toast.toast(t('bhyve.configSaved'));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    saving.value = false;
  }
}

async function saveGraphicsConfig() {
  saving.value = true;
  try {
    const payload = {};
    for (const key of graphicsKeyList) {
      if (key in config) payload[key] = String(config[key] ?? '');
    }
    await api.put(`/api/bhyve/vms/${encodeURIComponent(name)}`, { graphics: payload });
    toast.toast(t('bhyve.configSaved'));
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || t('common.operationFailed'));
  } finally {
    saving.value = false;
  }
}

async function saveOtherDevicesConfig() {
  saving.value = true;
  try {
    const payload = {};
    for (const [key, value] of Object.entries(config)) {
      if (isOtherDeviceKey(key)) payload[key] = String(value ?? '');
    }
    await api.put(`/api/bhyve/vms/${encodeURIComponent(name)}`, { other_devices: payload });
    toast.toast(t('bhyve.configSaved'));
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

  <div v-else>
    <WarnBanner :message="t('bhyve.configEditHint')" />
    <SectionCard v-model="activeTab" :tabs="sections">
      <template #default="{ active }">

        <!-- ── basic ── -->
        <template v-if="active === 'basic'">
          <div class="form-row"><label class="form-row-label">{{ t('bhyve.loader') }}</label><select v-model="config.loader"><option v-for="value in loaderOptions" :key="value" :value="value">{{ value }}</option></select></div>
          <div class="form-row"><label class="form-row-label">{{ t('bhyve.cpuCores') }}</label><input v-model="config.cpu" type="number" min="1" /></div>
          <div class="form-row"><label class="form-row-label">{{ t('bhyve.memory') }}</label><input v-model="config.memory" placeholder="1024M" /></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('uuid') }}</label><input v-model="config.uuid" /></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('bhyve_options') }}<FieldHelp :text="fieldHint('bhyve_options')" /></label><input v-model="config.bhyve_options" /></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('wired_memory') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('wired_memory')" @change="setBool('wired_memory', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('wired_memory') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('uefi_vars') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('uefi_vars')" @change="setBool('uefi_vars', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('uefi_vars') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('ignore_msr') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('ignore_msr')" @change="setBool('ignore_msr', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('ignore_msr') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('utctime') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('utctime')" @change="setBool('utctime', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('utctime') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('debug') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('debug')" @change="setBool('debug', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('debug') }}</span></label></div></div>
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('virt_random') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('virt_random')" @change="setBool('virt_random', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('virt_random') }}</span></label></div></div>

          <div class="card" style="margin-top:16px;">
            <h3 style="margin-bottom:16px;">{{ t('bhyve.editAdvanced') }}</h3>
            <div v-for="key in advancedKeys" :key="key" class="form-row" style="grid-template-columns: 280px 1fr;">
              <label class="form-row-label">{{ key }}</label>
              <div class="input-with-btn"><input v-model="config[key]" /><button type="button" class="btn-danger btn-sm fp-trigger" @click="remove(key)"><i class="fa-solid fa-trash"></i></button></div>
            </div>
            <div class="form-row" style="grid-template-columns: 280px 1fr;"><label class="form-row-label"><input v-model="newKey" :placeholder="t('bhyve.configKeyPlaceholder')" /></label><div class="input-with-btn"><input v-model="newValue" :placeholder="t('bhyve.configValuePlaceholder')" @keyup.enter="addAdvanced" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="addAdvanced"><i class="fa-solid fa-plus"></i></button></div></div>
          </div>

          <div class="form-actions-bar">
            <button type="button" :disabled="saving" @click="saveBasicConfig">{{ t('common.save') }}</button>
          </div>
        </template>

        <!-- ── disks ── -->
        <template v-if="active === 'disks'">
          <table v-if="disks.length">
            <thead><tr>
              <th>#</th>
              <th>{{ fieldLabel('type') }}</th>
              <th>{{ fieldLabel('dev') }}</th>
              <th>{{ fieldLabel('name') }}</th>
              <th>{{ t('common.size') }}</th>
              <th>{{ t('common.used') }}</th>
              <th>{{ fieldLabel('opts') }}</th>
              <th>{{ t('common.actions') }}</th>
            </tr></thead>
            <tbody>
              <tr v-for="disk in disks" :key="disk.index">
                <td class="mono">{{ disk.index }}</td>
                <td>{{ diskTypeLabel(disk.type) }}</td>
                <td>{{ disk.type === 'virtio-9p' ? '—' : diskDevLabel(disk.dev) }}</td>
                <td class="mono">
                  <template v-if="disk.type === 'virtio-9p'">{{ disk.ninepShare }} → {{ disk.ninepPath }}</template>
                  <template v-else>{{ diskDisplayName(disk) }}</template>
                </td>
                <td class="mono">{{ disk.size || '—' }}</td>
                <td class="mono">{{ disk.used || '—' }}</td>
                <td class="mono">{{ disk.opts || '—' }}</td>
                <td>
                  <div class="btn-group">
                    <button type="button" class="btn-secondary btn-sm" @click="openEditDisk(disk)"><i class="fa-solid fa-pen"></i></button>
                    <button type="button" class="btn-danger btn-sm" @click="confirmDeleteDisk(disk.index)"><i class="fa-solid fa-trash"></i></button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
          <div v-else class="empty">{{ t('bhyve.noDisks') }}</div>
          <div class="btn-group" style="margin-top:12px;">
            <button type="button" class="btn-secondary" @click="openImportDisk"><i class="fa-solid fa-download"></i> {{ t('bhyve.addDisk') }}</button>
            <button type="button" class="btn-secondary" @click="createDisk"><i class="fa-solid fa-plus"></i> {{ t('bhyve.createDisk') }}</button>
          </div>
        </template>

        <!-- ── networks ── -->
        <template v-if="active === 'networks'">
          <table v-if="networks.length">
            <thead><tr>
              <th>#</th>
              <th>{{ t('bhyve.fieldNetworkAdapter') }}</th>
              <th>{{ t('bhyve.fieldSwitch') }}</th>
              <th>MAC</th>
              <th>{{ t('common.actions') }}</th>
            </tr></thead>
            <tbody>
              <tr v-for="net in networks" :key="net.index">
                <td class="mono">{{ net.index }}</td>
                <td>{{ networkTypeLabel(net.type) }}</td>
                <td>{{ net.switch || '—' }}</td>
                <td class="mono">{{ net.mac || '—' }}</td>
                <td>
                  <div class="btn-group">
                    <button type="button" class="btn-secondary btn-sm" @click="openEditNetwork(net)"><i class="fa-solid fa-pen"></i></button>
                    <button type="button" class="btn-danger btn-sm" @click="confirmDeleteNetwork(net.index)"><i class="fa-solid fa-trash"></i></button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
          <div v-else class="empty">{{ t('bhyve.noNetworks') }}</div>
          <div style="margin-top:12px;">
            <button type="button" class="btn-secondary" @click="openAddNetwork"><i class="fa-solid fa-plus"></i> {{ t('bhyve.addNetwork') }}</button>
          </div>
        </template>

        <!-- ── graphics ── -->
        <template v-if="active === 'graphics'">
          <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('graphics')" @change="setBool('graphics', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('graphics') }}</span></label></div></div>
          <template v-if="boolValue('graphics')">
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_port') }}<FieldHelp :text="fieldHint('graphics_port')" /></label><input v-model="config.graphics_port" type="number" min="1" max="65535" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_listen') }}<FieldHelp :text="fieldHint('graphics_listen')" /></label><input v-model="config.graphics_listen" placeholder="0.0.0.0" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_res') }}<FieldHelp :text="fieldHint('graphics_res')" /></label><select v-model="config.graphics_res"><option value=""></option><option v-for="value in resolutionOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_wait') }}<FieldHelp :text="fieldHint('graphics_wait')" /></label><select v-model="config.graphics_wait"><option v-for="value in graphicsWaitOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('graphics_vga') }}<FieldHelp :text="fieldHint('graphics_vga')" /></label><select v-model="config.graphics_vga"><option v-for="value in graphicsVgaOptions" :key="value" :value="value">{{ value }}</option></select></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('vnc_password') }}<FieldHelp :text="fieldHint('vnc_password')" /></label><input v-model="config.vnc_password" type="password" /></div>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('xhci_mouse') }}</label><div><label class="checkbox-label"><input type="checkbox" :checked="boolValue('xhci_mouse')" @change="setBool('xhci_mouse', $event.target.checked)" /><span class="param-desc-inline">{{ fieldHint('xhci_mouse') }}</span></label></div></div>
          </template>
          <div class="form-actions-bar">
            <button type="button" :disabled="saving" @click="saveGraphicsConfig">{{ t('common.save') }}</button>
          </div>
        </template>

        <!-- ── other devices ── -->
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
          <div class="card" style="margin-bottom:12px;">
            <h3>{{ t('bhyve.editSound') }}</h3>
            <div class="form-row"><label class="form-row-label">{{ fieldLabel('sound') }}<FieldHelp :text="fieldHint('sound')" /></label><input type="checkbox" :checked="boolValue('sound')" @change="setBool('sound', $event.target.checked)" /></div>
            <template v-if="boolValue('sound')">
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('sound_play') }}</label><div class="input-with-btn"><input v-model="config.sound_play" placeholder="/dev/dsp0" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('sound_play')"><i class="fa-solid fa-folder-open"></i></button></div></div>
              <div class="form-row"><label class="form-row-label">{{ fieldLabel('sound_rec') }}</label><div class="input-with-btn"><input v-model="config.sound_rec" placeholder="/dev/dsp0" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="openPicker('sound_rec')"><i class="fa-solid fa-folder-open"></i></button></div></div>
            </template>
          </div>
          <div class="form-actions-bar">
            <button type="button" :disabled="saving" @click="saveOtherDevicesConfig">{{ t('common.save') }}</button>
          </div>
        </template>

      </template>
    </SectionCard>
  </div>

  <!-- ── disk edit/import modal ── -->
  <div v-if="diskModal.visible" class="modal-overlay" @click.self="diskModal.visible = false">
    <div class="modal">
      <h3 style="margin-top:0;">{{ diskModal.mode === 'edit' ? t('bhyve.editDiskTitle') : t('bhyve.importDiskTitle') }}</h3>
      <div class="form-row"><label class="form-row-label">{{ t('bhyve.fieldDeviceIndex') }}</label><input v-model.number="diskModal.index" type="number" min="0" /></div>
      <div class="form-row"><label class="form-row-label">{{ fieldLabel('type') }}</label><select v-model="diskModal.type" @change="onModalTypeChange"><option v-for="value in diskTypeOptions" :key="value" :value="value">{{ diskTypeLabel(value) }}</option></select></div>

      <template v-if="diskModal.type === 'virtio-9p'">
        <div class="form-row"><label class="form-row-label">{{ t('bhyve.ninepShare') }}</label><input v-model="diskModal.ninepShare" placeholder="data" /></div>
        <div class="form-row"><label class="form-row-label">{{ t('bhyve.ninepPath') }}</label><div class="input-with-btn"><input v-model="diskModal.ninepPath" placeholder="/vm/guest/data" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="pickerTarget = 'modal:ninepPath'"><i class="fa-solid fa-folder-open"></i></button></div></div>
      </template>
      <template v-else>
        <div class="form-row"><label class="form-row-label">{{ fieldLabel('dev') }}<FieldHelp :text="fieldHint('dev')" /></label><select v-model="diskModal.dev"><option v-for="value in diskDevOptions" :key="value" :value="value">{{ diskDevLabel(value) }}</option></select></div>
        <div class="form-row"><label class="form-row-label">{{ diskNameLabel(diskModal.dev) }}<FieldHelp :text="diskNameHint(diskModal.dev)" /></label>
          <select v-if="diskModal.dev === 'file'" v-model="diskModal.name"><option v-for="f in modalAvailableFiles()" :key="f" :value="f">{{ f }}</option></select>
          <select v-else-if="diskModal.dev === 'zvol'" v-model="diskModal.name"><option v-for="z in modalAvailableZvols()" :key="z" :value="z">{{ z }}</option></select>
          <div v-else-if="diskModal.dev === 'custom'" class="input-with-btn"><input v-model="diskModal.name" :placeholder="diskNamePlaceholder(diskModal.dev)" /><button type="button" class="btn-secondary btn-sm fp-trigger" @click="pickerTarget = 'modal:name'"><i class="fa-solid fa-folder-open"></i></button></div>
          <input v-else v-model="diskModal.name" :placeholder="diskNamePlaceholder(diskModal.dev)" />
        </div>
      </template>
      <div class="form-row"><label class="form-row-label">{{ fieldLabel('opts') }}<FieldHelp :text="fieldHint('opts')" /></label><input v-model="diskModal.opts" :placeholder="diskModal.type === 'virtio-9p' ? 'ro' : 'direct,nocache,ro'" /></div>

      <div class="btn-group" style="justify-content:flex-end; margin-top:16px;">
        <button type="button" class="btn-secondary" @click="diskModal.visible = false">{{ t('common.cancel') }}</button>
        <button type="button" :disabled="saving" @click="saveDiskModal">{{ t('common.save') }}</button>
      </div>
    </div>
  </div>

  <!-- ── network edit/add modal ── -->
  <div v-if="netModal.visible" class="modal-overlay" @click.self="netModal.visible = false">
    <div class="modal">
      <h3 style="margin-top:0;">{{ netModal.mode === 'edit' ? t('bhyve.editNetworkTitle') : t('bhyve.addNetworkTitle') }}</h3>
      <div class="form-row"><label class="form-row-label">{{ t('bhyve.fieldDeviceIndex') }}</label><input v-model.number="netModal.index" type="number" min="0" /></div>
      <div class="form-row"><label class="form-row-label">{{ t('bhyve.fieldNetworkAdapter') }}</label><select v-model="netModal.type"><option v-for="value in networkTypeOptions" :key="value" :value="value">{{ networkTypeLabel(value) }}</option></select></div>
      <div class="form-row"><label class="form-row-label">{{ t('bhyve.fieldSwitch') }}<FieldHelp :text="fieldHint('switch')" /></label><select v-model="netModal.switch"><option value=""></option><option v-for="sw in switches" :key="sw.name" :value="sw.name">{{ sw.name }}</option></select></div>
      <div class="form-row"><label class="form-row-label">{{ t('bhyve.fieldMacAddress') }}<FieldHelp :text="fieldHint('mac')" /></label><input v-model="netModal.mac" placeholder="58:9c:fc:00:00:01" /></div>

      <div class="btn-group" style="justify-content:flex-end; margin-top:16px;">
        <button type="button" class="btn-secondary" @click="netModal.visible = false">{{ t('common.cancel') }}</button>
        <button type="button" :disabled="saving" @click="saveNetModal">{{ t('common.save') }}</button>
      </div>
    </div>
  </div>

  <FilePicker v-if="pickerTarget" :mode="(pickerTarget === 'modal:ninepPath' || pickerTarget.startsWith('config:') && pickerTarget.endsWith('_path')) ? 'dir' : 'file'" :initial-path="pickerTarget === 'modal:name' ? diskModal.name || '/' : pickerTarget === 'modal:ninepPath' ? diskModal.ninepPath || '/' : config[pickerTarget] || '/'" @select="onPickerSelect" @close="pickerTarget = ''" />
</template>
