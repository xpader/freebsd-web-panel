// DNS — show and manage system DNS configuration from /etc/resolv.conf.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { t } from '../i18n/index.js';

let _cfg = null;

export async function renderDns(app) {
  renderLayout(app, '/network/dns', `
    <div class="page-header">
      <h1>${t('dns.title')}</h1>
      <p>${t('dns.subtitle')}</p>
    </div>
    <div class="toolbar">
      <span></span>
      <div></div>
      <button onclick="window.__fwpDnsApply()">${t('common.apply')}</button>
    </div>
    <div id="dns-content">
      <div class="card" style="padding:1rem;"><span class="spinner"></span> ${t('common.loading')}</div>
    </div>
  `);

  await load();
}

async function load() {
  const container = document.getElementById('dns-content');
  try {
    _cfg = await api.get('/api/network/dns');
  } catch (err) {
    container.innerHTML = `<div class="card" style="padding:1rem;">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
    return;
  }
  renderConfig();
}

function renderConfig() {
  const container = document.getElementById('dns-content');
  const slots = [0, 1, 2].map(i => _cfg.nameservers[i] || '');

  let html = `
    <div class="card">
      <div class="dns-slots">
        ${slots.map((ns, i) => `
          <div class="dns-slot">
            <label class="dns-slot-label">NameServer ${i + 1}</label>
            <div class="dns-slot-input">
              <input type="text" id="dns-ns-${i}" class="dns-input mono" value="${escAttr(ns)}" placeholder="${i === 0 ? '8.8.8.8' : ''}" />
              <button class="btn-secondary btn-sm dns-slot-clear" onclick="window.__fwpDnsClear(${i})" title="${t('common.clear')}"><i class="fa-solid fa-xmark"></i></button>
            </div>
            <button class="btn-secondary btn-sm" ${i === 0 ? 'disabled' : ''} onclick="window.__fwpDnsSwap(${i}, -1)" title="${t('net.up')}"><i class="fa-solid fa-arrow-up"></i></button>
            <button class="btn-secondary btn-sm" ${i === 2 ? 'disabled' : ''} onclick="window.__fwpDnsSwap(${i}, 1)" title="${t('net.down')}"><i class="fa-solid fa-arrow-down"></i></button>
          </div>`).join('')}
      </div>
    </div>`;

  const kvRows = [];
  if (_cfg.domain) {
    kvRows.push(`<div class="kv"><span class="kv-key">${t('dns.domain')}</span><span class="kv-val mono">${esc(_cfg.domain)}</span></div>`);
  }
  if (_cfg.search.length) {
    kvRows.push(`<div class="kv"><span class="kv-key">${t('dns.search')}</span><span class="kv-val mono">${esc(_cfg.search.join(', '))}</span></div>`);
  }
  if (_cfg.options.length) {
    kvRows.push(`<div class="kv"><span class="kv-key">${t('dns.options')}</span><span class="kv-val mono">${esc(_cfg.options.join(', '))}</span></div>`);
  }

  if (kvRows.length) {
    html += `<div class="card" style="margin-top:16px;">${kvRows.join('')}</div>`;
  }

  container.innerHTML = html;
}

function collectServers() {
  return [0, 1, 2].map(i => {
    const el = document.getElementById(`dns-ns-${i}`);
    return el ? el.value.trim() : '';
  });
}

window.__fwpDnsClear = (i) => {
  const el = document.getElementById(`dns-ns-${i}`);
  if (el) el.value = '';
};

window.__fwpDnsSwap = (i, dir) => {
  const j = i + dir;
  if (j < 0 || j > 2) return;
  const a = document.getElementById(`dns-ns-${i}`);
  const b = document.getElementById(`dns-ns-${j}`);
  if (a && b) {
    const tmp = a.value;
    a.value = b.value;
    b.value = tmp;
  }
};

window.__fwpDnsApply = async () => {
  const servers = collectServers();
  // Front-end validation: non-empty entries must be valid IPs.
  for (const s of servers) {
    if (s && !isValidIp(s)) {
      toast(t('dns.invalidIp', { addr: s }), 'error');
      return;
    }
  }
  // Check duplicates.
  const filled = servers.filter(s => s);
  if (new Set(filled).size !== filled.length) {
    toast(t('dns.duplicate'), 'error');
    return;
  }

  api.put('/api/network/dns/nameservers', { nameservers: servers }).then((cfg) => {
    _cfg = cfg;
    toast(t('common.saved'));
    renderConfig();
  }).catch((e) => toast(e.message || t('common.saveFailed', { msg: '' }), 'error'));
};

function isValidIp(s) {
  // IPv4: 4 octets 0-255
  if (/^(\d{1,3}\.){3}\d{1,3}$/.test(s)) {
    return s.split('.').every(o => Number(o) >= 0 && Number(o) <= 255);
  }
  // IPv6: must contain ':' and only valid hex chars
  return /^[0-9a-fA-F:]+$/.test(s) && s.includes(':');
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

function escAttr(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/'/g, '&#39;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}
