// Web terminal page — xterm.js over a WebSocket bridged to a server-side PTY.
//
// Loaded only on the /shell route. The xterm UMD bundle and its CSS are injected
// on demand (not on every page) and reused if already present.

import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

const XTERT_CSS = '/vendor/xterm/xterm.css';
const XTERM_JS = '/vendor/xterm/xterm.js';
const FIT_JS = '/vendor/xterm/xterm-addon-fit.js';

let alive = false;     // guards against duplicate setup if route re-renders
let cleanup = null;    // current session teardown, invoked on navigation away

export async function renderTerminal(app) {
  renderLayout(app, '/shell', `
    <div class="page-header">
      <h1>${t('term.title')}</h1>
      <p>${t('term.subtitle')}</p>
    </div>
    <div class="term-page">
      <div class="term-toolbar">
        <span id="term-status" class="badge badge-dim">${t('common.loading')}</span>
        <button id="term-reconnect" class="btn btn-sm" disabled>${t('term.reconnect')}</button>
      </div>
      <div id="term-host" class="term-host"></div>
    </div>
  `);

  // Lock the layout: make the main area a full-height flex column and hide
  // the page scrollbar behind the terminal while the page is mounted.
  document.body.classList.add('term-active');

  try {
    await loadAssets();
  } catch (err) {
    const host = document.getElementById('term-host');
    if (host) host.innerHTML = `<div class="empty">${t('term.loadFailed', { msg: esc(err.message || String(err)) })}</div>`;
    return;
  }

  startSession();
}

function startSession() {
  // Tear down any previous session bound to this page instance.
  if (cleanup) { try { cleanup(); } catch {} cleanup = null; }

  const Terminal = window.Terminal;
  const FitAddon = window.FitAddon?.FitAddon;
  const host = document.getElementById('term-host');
  if (!host || !Terminal || !FitAddon) return;

  const term = new Terminal({
    cursorBlink: true,
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace',
    fontSize: 13,
    scrollback: 5000,
    theme: {
      background: '#0b0e14',
      foreground: '#d6dbe5',
      cursor: '#d6dbe5',
      selectionBackground: '#264f78aa',
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(host);
  try { fit.fit(); } catch {}

  const statusEl = document.getElementById('term-status');
  const reconnectBtn = document.getElementById('term-reconnect');
  let resizeTimer = null;

  const token = sessionStorage.getItem('fwp_token');
  if (!token) return;

  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${proto}//${location.host}/api/term/ws?token=${encodeURIComponent(token)}`;
  const ws = new WebSocket(url);

  const sendSize = () => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    }
  };

  const dataDisposer = term.onData((data) => {
    if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: 'input', data }));
  });
  const resizeDisposer = term.onResize(() => sendSize());

  // Resize when the host element changes size (window/panel resize).
  const ro = new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => { try { fit.fit(); } catch {} }, 80);
  });
  ro.observe(host);

  const setStatus = (cls, text) => {
    if (statusEl) { statusEl.className = `badge ${cls}`; statusEl.textContent = text; }
  };

  ws.onopen = () => {
    setStatus('badge-success', t('term.connected'));
    if (reconnectBtn) reconnectBtn.disabled = true;
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
      if (reconnectBtn) reconnectBtn.disabled = false;
    }
  };
  ws.onerror = () => setStatus('badge-danger', t('term.error'));
  ws.onclose = () => {
    setStatus('badge-dim', t('term.disconnected'));
    if (reconnectBtn) reconnectBtn.disabled = false;
  };

  if (reconnectBtn) {
    reconnectBtn.onclick = () => {
      term.reset();
      startSession();
    };
  }

  cleanup = () => {
    document.body.classList.remove('term-active');
    try { dataDisposer.dispose(); } catch {}
    try { resizeDisposer.dispose(); } catch {}
    try { ro.disconnect(); } catch {}
    clearTimeout(resizeTimer);
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) ws.close();
    try { term.dispose(); } catch {}
  };
}

// --- asset loading (idempotent) ---

function loadCss(href) {
  if (document.querySelector(`link[href="${href}"]`)) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = href;
    link.onload = resolve;
    link.onerror = () => reject(new Error(href));
    document.head.appendChild(link);
  });
}

function loadScript(src) {
  if (document.querySelector(`script[src="${src}"]`)) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = src;
    s.onload = resolve;
    s.onerror = () => reject(new Error(src));
    document.head.appendChild(s);
  });
}

async function loadAssets() {
  await Promise.all([loadCss(XTERT_CSS), loadScript(XTERM_JS), loadScript(FIT_JS)]);
}

// Global teardown: when the hash changes away from /shell, free the PTY + DOM.
if (!alive) {
  alive = true;
  window.addEventListener('hashchange', () => {
    if (location.hash !== '#/shell' && cleanup) {
      try { cleanup(); } catch {}
      cleanup = null;
    }
  });
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
