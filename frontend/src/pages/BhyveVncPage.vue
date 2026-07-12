<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import RFB from '@novnc/novnc';
import BackButton from '../components/ui/BackButton.vue';

const { t } = useI18n();
const route = useRoute();
const name = route.params.name;
const isStandalone = computed(() => route.query.standalone === '1');

const statusText = ref(t('bhyve.vncConnecting'));
const statusClass = ref('badge badge-dim');
const vncScreenRef = ref(null);

let rfb = null;

function setStatus(cls, text) {
  statusClass.value = `badge ${cls}`;
  statusText.value = text;
}

function connect() {
  const token = sessionStorage.getItem('fwp_token');
  if (!token) {
    setStatus('badge-danger', t('common.unauthenticated'));
    return;
  }
  if (!vncScreenRef.value) return;

  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${proto}//${location.host}/api/bhyve/vms/${encodeURIComponent(name)}/vnc?token=${encodeURIComponent(token)}`;

  rfb = new RFB(vncScreenRef.value, url, {
    wsProtocols: ['binary'],
  });

  rfb.addEventListener('connect', () => {
    setStatus('badge-success', t('term.connected'));
    rfb.focus();
  });
  rfb.addEventListener('disconnect', (e) => {
    if (e.detail.clean) {
      setStatus('badge-dim', t('term.disconnected'));
    } else {
      setStatus('badge-danger', t('term.error'));
    }
    rfb = null;
  });
  rfb.addEventListener('credentialsrequired', () => {
    setStatus('badge-warn', 'Password required');
  });
  rfb.scaleViewport = true;
  rfb.resizeSession = false;
}

function disconnect() {
  if (rfb) {
    rfb.disconnect();
    rfb = null;
  }
}

function reconnect() {
  disconnect();
  if (vncScreenRef.value) vncScreenRef.value.innerHTML = '';
  setStatus('badge-dim', t('bhyve.vncConnecting'));
  connect();
}

function openStandalone() {
  window.open(`#${route.path}?standalone=1`, '_blank', 'width=1024,height=768');
  disconnect();
  setStatus('badge-dim', t('term.disconnected'));
}

onMounted(connect);
onUnmounted(disconnect);
</script>

<template>
  <div class="page-header">
    <div class="flex">
      <BackButton v-if="!isStandalone" :href="`#/bhyve/detail/${name}`" />
      <h1>VNC: {{ name }}</h1>
    </div>
    <div class="flex btn-group" style="margin-left:auto;">
      <span :class="statusClass">{{ statusText }}</span>
      <button v-if="!isStandalone" class="btn btn-sm" @click="openStandalone"><i class="fa-solid fa-up-right-from-square"></i> {{ t('term.openInNewWindow') }}</button>
      <button class="btn btn-sm" @click="reconnect">{{ t('term.reconnect') }}</button>
    </div>
  </div>
  <div class="vnc-page">
    <div ref="vncScreenRef" class="vnc-screen"></div>
  </div>
</template>

<style scoped>
.vnc-page {
  background: #000;
  border-radius: var(--radius);
  overflow: hidden;
  min-height: 400px;
}
.vnc-screen {
  width: 100%;
  height: calc(100vh - 120px);
  min-height: 400px;
}
.vnc-screen :deep(canvas) {
  display: block;
  margin: 0 auto;
}
</style>
