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

// ASCII frame opcodes (see docs/impl/14-terminal.md): 1 printable char + payload.
// 'i'/'r' c→s; 'o'/'x'/'e' s→c — letters never shared, dumps self-describe.
const chr = (c) => c.charCodeAt(0);
const OP = { INPUT: chr('i'), RESIZE: chr('r'), OUTPUT: chr('o'), EXIT: chr('x'), ERROR: chr('e') };
const enc = new TextEncoder();
const dec = new TextDecoder();

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
let binDisposer = null;

function setStatus(cls, text) {
  statusClass.value = `badge ${cls}`;
  statusText.value = text;
}

// Copy via the legacy execCommand path — unlike navigator.clipboard it needs
// no secure context, only transient user activation. OSC 52 copies usually
// run within milliseconds of the keystroke/mouse event that triggered the
// yank, so activation is present and this succeeds even on plain HTTP.
// Returns false when activation is missing/expired (e.g. a program writing
// the clipboard long after the last interaction).
function copyViaExecCommand(text) {
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.readOnly = true;
  ta.style.position = 'fixed';
  ta.style.left = '-9999px';
  ta.style.top = '0';
  document.body.appendChild(ta);
  ta.focus();
  ta.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch { ok = false; }
  ta.remove();
  term?.focus();
  return ok;
}

// Copy to the system clipboard. Preference chain:
//   1. navigator.clipboard.writeText (secure contexts only)
//   2. document.execCommand('copy')  (works on plain HTTP while user
//      activation from the triggering keystroke/mouse event is still valid)
// Total failure (no activation, e.g. a program writing the clipboard long
// after the last interaction) is logged to the console only.
function copyToClipboard(text) {
  const logFail = (reason, extra) => console.warn('[fwp-term] clipboard copy failed',
    { bytes: text.length, protocol: location.protocol, isSecureContext, reason, ...extra });
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).catch((err) => {
      console.debug('[fwp-term] clipboard API rejected, trying execCommand fallback', err);
      if (!copyViaExecCommand(text)) logFail('clipboard API rejected and execCommand copy failed (no user activation?)', { error: err });
    });
    return;
  }
  console.debug('[fwp-term] clipboard API unavailable (insecure context), trying execCommand fallback');
  if (!copyViaExecCommand(text)) {
    logFail('clipboard API unavailable and execCommand copy failed (no user activation?)',
      { hint: 'navigator.clipboard requires a secure context — access via https or localhost' });
  }
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

  // Clipboard wiring — xterm.js core deliberately leaves all three clipboard
  // paths to the host app (paste key, copy key, OSC 52).
  // Paste: return false without preventDefault → xterm skips the keydown and
  // the browser's native paste lands on xterm's hidden textarea (the same
  // paste-event path as the right-click menu). Without this Ctrl+V becomes ^V.
  // Copy (Windows Terminal semantics): Ctrl+C with an active selection →
  // return false so the native copy event reaches xterm's copyHandler; without
  // a selection Ctrl+C still sends ^C (SIGINT). Cmd+C without a selection is
  // swallowed to stop the native copy event from clearing the clipboard, and
  // Ctrl+Shift+C copies via the clipboard API (Chrome may reserve it for
  // DevTools on some platforms).
  term.attachCustomKeyEventHandler((ev) => {
    if (ev.type !== 'keydown') return true;
    const k = ev.key.toLowerCase();
    if ((ev.ctrlKey || ev.metaKey) && k === 'v') return false;
    if (ev.ctrlKey && ev.shiftKey && k === 'c') {
      if (term.hasSelection()) copyToClipboard(term.getSelection());
      return false;
    }
    if ((ev.ctrlKey || ev.metaKey) && !ev.shiftKey && k === 'c') {
      if (term.hasSelection()) return false;
      if (ev.metaKey) { ev.preventDefault(); return false; }
      return true; // Ctrl+C without selection → ^C to the PTY
    }
    return true;
  });

  // OSC 52: programs (vim/tmux/remote ssh yanks) write the host clipboard via
  // `ESC]52;c;<base64>ESC\`. xterm.js has no built-in handler, so the sequence
  // is silently dropped — register one. The `?` query form is ignored so the
  // clipboard can never be read back through the terminal. On failure (plain
  // HTTP has no clipboard API; some browsers reject gesture-less writes) the
  // content is shown in a dialog for manual selection-copy instead.
  term.parser.registerOscHandler(52, (data) => {
    const b64 = data.split(';')[1];
    if (b64 && b64 !== '?') {
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      copyToClipboard(dec.decode(bytes));
    }
    return true;
  });

  document.body.classList.add('term-active');

  const token = sessionStorage.getItem('fwp_token');
  if (!token) return;

  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const qs = new URLSearchParams({ token, ...props.query });
  ws = new WebSocket(`${proto}//${location.host}/api/term/ws?${qs}`);
  ws.binaryType = 'arraybuffer'; // binary protocol — avoid per-frame Blob promises

  const sendSize = () => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(enc.encode(`r${term.cols},${term.rows}`));
    }
  };

  dataDisposer = term.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(enc.encode('i' + data));
  });
  // Non-UTF-8 input (e.g. X10 mouse reports): each charCode is one raw byte.
  binDisposer = term.onBinary((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      const frame = new Uint8Array(1 + data.length);
      frame[0] = OP.INPUT;
      for (let i = 0; i < data.length; i++) frame[i + 1] = data.charCodeAt(i) & 0xff;
      ws.send(frame);
    }
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
    const u8 = new Uint8Array(e.data);
    if (!u8.length) return;
    if (u8[0] === OP.OUTPUT) term.write(u8.subarray(1));
    else if (u8[0] === OP.ERROR) term.write(`\r\n\x1b[31m${dec.decode(u8.subarray(1))}\x1b[0m\r\n`);
    else if (u8[0] === OP.EXIT) {
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
  try { binDisposer?.dispose(); } catch {}
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
