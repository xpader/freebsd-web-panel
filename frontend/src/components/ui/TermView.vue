<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { termTheme, TERM_FONT } from '../../lib/term-theme.js';
import { effective as themeEff } from '../../stores/theme.js';
import BackButton from './BackButton.vue';

const props = defineProps({
  // WS 额外查询参数，如 { jail: name } / { vm: name }
  query: { type: Object, default: () => ({}) },
  // 返回链接；为空则不渲染返回按钮行
  back: { type: String, default: '' },
});

const { t } = useI18n();
const route = useRoute();
const isStandalone = computed(() => route.query.standalone === '1');
const statusText = ref(t('common.loading'));
const statusClass = ref('badge badge-dim');
const reconnectDisabled = ref(true);
const termHostRef = ref(null);

let term = null;
let fit = null;
let ws = null;
let ro = null;
let resizeTimer = null;
let dataDisposer = null;
let resizeDisposer = null;

function setStatus(cls, text) {
  statusClass.value = `badge ${cls}`;
  statusText.value = text;
}

function startSession() {
  if (term) { cleanup(); }

  if (!termHostRef.value) return;

  term = new Terminal({
    cursorBlink: true,
    fontFamily: TERM_FONT,
    fontSize: 13,
    scrollback: 5000,
    theme: termTheme(themeEff.value),
  });
  fit = new FitAddon();
  term.loadAddon(fit);
  term.open(termHostRef.value);
  try { fit.fit(); } catch {}

  document.body.classList.add('term-active');

  const token = sessionStorage.getItem('fwp_token');
  if (!token) return;

  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const qs = new URLSearchParams({ token, ...props.query });
  ws = new WebSocket(`${proto}//${location.host}/api/term/ws?${qs}`);

  const sendSize = () => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    }
  };

  dataDisposer = term.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: 'input', data }));
  });
  resizeDisposer = term.onResize(() => sendSize());

  ro = new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => { try { fit.fit(); } catch {} }, 80);
  });
  ro.observe(termHostRef.value);

  ws.onopen = () => {
    setStatus('badge-success', t('term.connected'));
    reconnectDisabled.value = true;
    sendSize();
    term.focus();
  };
  ws.onmessage = (e) => {
    let msg;
    try { msg = JSON.parse(e.data); } catch { return; }
    if (msg.type === 'output') term.write(msg.data ?? '');
    else if (msg.type === 'error') term.write(`\r\n\x1b[31m${msg.data}\x1b[0m\r\n`);
    else if (msg.type === 'exit') {
      term.write(`\r\n\x1b[2m[${t('term.ended')}]\x1b[0m\r\n`);
      setStatus('badge-dim', t('term.disconnected'));
      reconnectDisabled.value = false;
    }
  };
  ws.onerror = () => setStatus('badge-danger', t('term.error'));
  ws.onclose = () => {
    setStatus('badge-dim', t('term.disconnected'));
    reconnectDisabled.value = false;
  };
}

function reconnect() {
  if (term) term.reset();
  startSession();
}

function openStandalone() {
  window.open(`#${route.path}?standalone=1`, '_blank', 'width=1024,height=768');
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) ws.close();
  setStatus('badge-dim', t('term.disconnected'));
  reconnectDisabled.value = false;
}

function cleanup() {
  document.body.classList.remove('term-active');
  try { dataDisposer?.dispose(); } catch {}
  try { resizeDisposer?.dispose(); } catch {}
  try { ro?.disconnect(); } catch {}
  clearTimeout(resizeTimer);
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) ws.close();
  try { term?.dispose(); } catch {}
  term = null;
}

onMounted(() => {
  startSession();
});

onUnmounted(() => {
  cleanup();
});

watch(themeEff, (val) => { if (term) term.options.theme = termTheme(val); });
</script>

<template>
  <div class="page-header">
    <div v-if="back" class="flex">
      <BackButton v-if="!isStandalone" :href="back" />
      <slot name="title" />
    </div>
    <slot v-else name="title" />
    <div class="flex btn-group" style="margin-left:auto;">
      <span :class="statusClass">{{ statusText }}</span>
      <button v-if="!isStandalone" class="btn btn-sm" @click="openStandalone"><i class="fa-solid fa-up-right-from-square"></i> {{ t('term.openInNewWindow') }}</button>
      <button class="btn btn-sm" :disabled="reconnectDisabled" @click="reconnect">{{ t('term.reconnect') }}</button>
    </div>
  </div>
  <div class="term-page">
    <div ref="termHostRef" class="term-host"></div>
  </div>
</template>
