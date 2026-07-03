// Jails — list running jail containers and view details.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

// ===== List page =====

export async function renderJailsRunning(app) {
  renderLayout(app, '/jails/running', `
    <div class="page-header">
      <h1>${t('jails.title')}</h1>
      <p>${t('jails.subtitle')}</p>
    </div>
    <div class="toolbar">
      <span id="jail-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpJailReload()">${t('common.refresh')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>JID</th>
          <th>${t('common.name')}</th>
          <th>${t('jails.hostname')}</th>
          <th>${t('jails.path')}</th>
          <th>IP</th>
          <th>${t('common.status')}</th>
        </tr></thead>
        <tbody id="jail-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);
  await loadJails();
}

async function loadJails() {
  const tbody = document.getElementById('jail-tbody');
  const countEl = document.getElementById('jail-count');
  try {
    const jails = await api.get('/api/jails');
    if (countEl) countEl.textContent = t('jails.count', { n: jails.length });
    if (!jails.length) {
      tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('jails.noJails')}</td></tr>`;
      return;
    }
    tbody.innerHTML = jails.map((j) => `
      <tr class="row-clickable" onclick="location.hash='#/jails/detail/${escAttr(j.name)}'">
        <td class="mono">${j.jid}</td>
        <td class="mono"><strong>${esc(j.name)}</strong></td>
        <td>${esc(j.hostname || '—')}</td>
        <td class="mono text-dim">${esc(j.path || '—')}</td>
        <td class="mono">${formatIps(j.ip4_addr, j.ip6_addr)}</td>
        <td>${stateBadge(j.state)}</td>
      </tr>`).join('');
  } catch (err) {
    if (countEl) countEl.textContent = '';
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
  }
}

// ===== Detail page =====

export async function renderJailDetail(app, hashPath) {
  const name = hashPath.replace(/^\/jails\/detail\//, '');

  renderLayout(app, '/jails/running', `
    <div class="page-header">
      <div class="flex">
        <a href="#/jails/running" class="btn-secondary btn-sm">${t('common.navBack')}</a>
        <h1>${esc(name)}</h1>
      </div>
      <p>${t('jails.detailSubtitle')}</p>
    </div>
    <div id="jail-detail"><div class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>
  `);

  const el = document.getElementById('jail-detail');
  let d;
  try {
    d = await api.get(`/api/jails/${encodeURIComponent(name)}`);
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
    return;
  }

  const allowEntries = Object.entries(d.params)
    .filter(([k]) => k.startsWith('allow.'))
    .sort(([a], [b]) => a.localeCompare(b));

  const otherEntries = Object.entries(d.params)
    .filter(([k]) => !k.startsWith('allow.'))
    .sort(([a], [b]) => a.localeCompare(b));

  el.innerHTML = `
    <div class="stat-grid">
      <div class="card"><div class="card-title">JID</div><div class="card-value sm">${d.jid}</div></div>
      <div class="card"><div class="card-title">${t('common.status')}</div><div class="card-value sm">${stateBadge(d.state)}</div></div>
      <div class="card"><div class="card-title">persist</div><div class="card-value sm">${boolBadge(d.persist)}</div></div>
      <div class="card"><div class="card-title">${t('jails.parent')}</div><div class="card-value sm">${d.params.parent || '0'}</div></div>
    </div>

    <div class="card">
      <div class="card-title">${t('common.network')}</div>
      <table class="kv-table">
        ${kvRow('ip4.addr', d.ip4_addr.length ? d.ip4_addr.join(', ') : '—')}
        ${kvRow('ip4.saddrsel', d.params['ip4.saddrsel'] || '—')}
        ${kvRow('ip6.addr', d.ip6_addr.length ? d.ip6_addr.join(', ') : '—')}
        ${kvRow('ip6.saddrsel', d.params['ip6.saddrsel'] || '—')}
        ${kvRow('vnet', d.params.vnet || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.hostInfo')}</div>
      <table class="kv-table">
        ${kvRow('host.hostname', d.hostname || '—')}
        ${kvRow('host.domainname', d.params['host.domainname'] || '—')}
        ${kvRow('host.hostuuid', d.params['host.hostuuid'] || '—')}
        ${kvRow('host.hostid', d.params['host.hostid'] || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.security')}</div>
      <table class="kv-table">
        ${kvRow('securelevel', d.params.securelevel || '—')}
        ${kvRow('enforce_statfs', d.params.enforce_statfs || '—')}
        ${kvRow('devfs_ruleset', d.params.devfs_ruleset || '—')}
        ${kvRow('children.max', d.params['children.max'] || '—')}
        ${kvRow('children.cur', d.params['children.cur'] || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.system')}</div>
      <table class="kv-table">
        ${kvRow('osrelease', d.params.osrelease || '—')}
        ${kvRow('osreldate', d.params.osreldate || '—')}
        ${kvRow('cpuset.id', d.params['cpuset.id'] || '—')}
        ${kvRow('path', d.path || '—')}
      </table>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.permissions')}</div>
      <div class="perm-grid">
        ${allowEntries.map(([k, v]) => permBadge(k.replace(/^allow\./, ''), v === 'true')).join('')}
      </div>
    </div>

    <div class="card">
      <div class="card-title">${t('jails.allParams')}</div>
      <table class="kv-table">
        ${otherEntries.map(([k, v]) => kvRow(k, v || '—')).join('')}
      </table>
    </div>`;
}

// ===== Helpers =====

function formatIps(ip4, ip6) {
  const all = [...(ip4 || []), ...(ip6 || [])];
  if (!all.length) return '<span class="text-dim">—</span>';
  return all.map((ip) => `<span class="badge badge-dim">${esc(ip)}</span>`).join(' ');
}

function stateBadge(state) {
  if (state === 'dying') return `<span class="badge badge-warn">${t('jails.dying')}</span>`;
  return `<span class="badge badge-success">${t('jails.running')}</span>`;
}

function boolBadge(val) {
  return val
    ? `<span class="badge badge-success">${t('common.enabled')}</span>`
    : `<span class="badge badge-dim">${t('common.disabled')}</span>`;
}

function permBadge(name, allowed) {
  const cls = allowed ? 'badge-success' : 'badge-dim';
  return `<span class="badge ${cls}">${esc(name)}</span>`;
}

function kvRow(key, val) {
  return `<tr><td class="mono text-dim">${esc(key)}</td><td class="mono">${esc(val)}</td></tr>`;
}

window.__fwpJailReload = () => {
  const tbody = document.getElementById('jail-tbody');
  if (tbody) tbody.innerHTML = `<tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>`;
  loadJails();
};

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
