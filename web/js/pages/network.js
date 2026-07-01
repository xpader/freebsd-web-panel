// Network Interfaces — list interfaces, routes, and default gateway.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { t } from '../i18n/index.js';

let _interfaces = [];
let _routes = [];
let _gateway = null;

export async function renderNetwork(app) {
  renderLayout(app, '/network', `
    <div class="page-header">
      <h1>${t('net.title')}</h1>
      <p>${t('net.subtitle')}</p>
    </div>
    <div class="toolbar">
      <span id="net-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpNetRefresh()">${t('common.refresh')}</button>
    </div>

    <div id="net-interfaces">
      <div class="card" style="padding:1rem;"><span class="spinner"></span> ${t('common.loading')}</div>
    </div>

    <div id="net-gateway-section"></div>

    <h2 style="margin-top:2rem;">${t('net.routes')}</h2>
    <div id="net-routes-container">
      <div class="card" style="padding:0;">
        <table>
          <thead><tr><th>${t('net.family')}</th><th>${t('net.destination')}</th><th>${t('net.gateway')}</th><th>${t('common.status')}</th><th>${t('common.device')}</th></tr></thead>
          <tbody id="net-routes-tbody">
            <tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
          </tbody>
        </table>
      </div>
    </div>
  `);

  await loadData();
}

async function loadData() {
  try {
    [_interfaces, _routes, _gateway] = await Promise.all([
      api.get('/api/network/interfaces'),
      api.get('/api/network/routes'),
      api.get('/api/network/gateway'),
    ]);
  } catch (err) {
    document.getElementById('net-interfaces').innerHTML =
      `<div class="card" style="padding:1rem;">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
    return;
  }
  renderInterfaces();
  renderGateway();
  renderRoutes();
}

function renderInterfaces() {
  const container = document.getElementById('net-interfaces');
  const countEl = document.getElementById('net-count');
  if (countEl) countEl.textContent = `${_interfaces.length} ${t('common.device')}`;

  if (!_interfaces.length) {
    container.innerHTML = `<div class="card" style="padding:1rem;">${t('net.noInterfaces')}</div>`;
    return;
  }

  const physical = _interfaces.filter(i => i.is_physical);
  const others = _interfaces.filter(i => !i.is_physical);

  let html = '';
  if (physical.length) {
    html += `<h3>${t('net.physical')}</h3>`;
    html += `<div class="card-grid">${physical.map(renderCard).join('')}</div>`;
  }
  if (others.length) {
    html += `<h3 style="margin-top:${physical.length ? '2rem' : '0'};">${t('net.virtual')}</h3>`;
    html += `<div class="card-grid">${others.map(renderCard).join('')}</div>`;
  }
  container.innerHTML = html;
}

function renderCard(iface) {
  const iconClass = iface.is_up ? 'up' : 'down';
  const iconName = iface.is_loopback ? 'fa-rotate' : 'fa-ethernet';
  const linkLabel = iface.link_state === 'up' ? t('net.linkUp')
    : iface.link_state === 'down' ? t('net.linkDown')
    : t('common.unknown');

  const ipv4Rows = iface.ipv4.map(ip =>
    `<div class="${ip.is_alias ? 'text-dim' : ''}">
      ${esc(ip.address)}${ip.prefix_len != null ? `/${ip.prefix_len}` : ''}
      ${ip.is_alias ? ` <span class="badge">${t('net.alias')}</span>` : ''}
    </div>`
  ).join('') || '<span class="text-dim">—</span>';

  const ipv6Rows = iface.ipv6.map(ip =>
    `<div class="${ip.is_alias ? 'text-dim' : ''}">
      ${esc(ip.address)}${ip.prefix_len != null ? `/${ip.prefix_len}` : ''}
      ${ip.is_alias ? ` <span class="badge">${t('net.alias')}</span>` : ''}
    </div>`
  ).join('') || '<span class="text-dim">—</span>';

  return `
    <div class="card net-iface">
      <div class="net-iface-header">
        <i class="fa-solid ${iconName} net-iface-icon ${iconClass}"></i>
        <span class="net-iface-name mono">${esc(iface.name)}</span>
        <span class="net-iface-name-spacer"></span>
        <span class="badge">${esc(linkLabel)}</span>
      </div>
      <div class="net-iface-body">
        <div class="kv"><span class="kv-key">IPv4</span><span class="kv-val">${ipv4Rows}</span></div>
        <div class="kv"><span class="kv-key">IPv6</span><span class="kv-val">${ipv6Rows}</span></div>
        <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">${esc(iface.mac || '—')}</span></div>
        ${iface.is_physical && iface.baudrate ? `<div class="kv"><span class="kv-key">${t('net.speed')}</span><span class="kv-val">${fmtSpeed(iface.baudrate)}</span></div>` : ''}
        ${iface.groups.length ? `<div class="kv"><span class="kv-key">${t('net.groups')}</span><span class="kv-val">${iface.groups.map(g => `<span class="badge badge-dim">${esc(g)}</span>`).join(' ')}</span></div>` : ''}
      </div>
      <div class="net-iface-footer">
        <button class="btn-secondary btn-sm" onclick="window.__fwpNetDetail('${escAttr(iface.name)}')">${t('net.detail')}</button>
      </div>
    </div>`;
}

function renderGateway() {
  const container = document.getElementById('net-gateway-section');
  if (!container) return;
  const gw = _gateway;
  if (!gw) { container.innerHTML = ''; return; }

  const gwVal = gw.gateway ? `<strong class="mono">${esc(gw.gateway)}</strong>` : `<span class="text-dim">${t('net.notConfigured')}</span>`;
  const ifVal = gw.interface ? `(${esc(gw.interface)})` : '';
  const cfgVal = gw.configured ? `<span class="mono">${esc(gw.configured)}</span>` : `<span class="text-dim">${t('net.notConfigured')}</span>`;

  container.innerHTML = `
    <h2 style="margin-top:2rem;">${t('net.defaultGateway')}</h2>
    <div class="card" style="padding:1rem;">
      <div class="kv"><span class="kv-key">${t('net.defaultGateway')}</span><span class="kv-val">${gwVal} ${ifVal}</span></div>
      <div class="kv"><span class="kv-key">${t('net.gatewayConfigured')}</span><span class="kv-val">${cfgVal}</span></div>
    </div>`;
}

function renderRoutes() {
  const container = document.getElementById('net-routes-container');
  if (!container) return;

  if (!_routes.length) {
    container.innerHTML = `<div class="card" style="padding:1rem;">${t('common.noData')}</div>`;
    return;
  }

  const v4 = _routes.filter(r => r.family === 'Internet');
  const v6 = _routes.filter(r => r.family === 'Internet6');
  const cols = 5;

  let body = '';
  for (const [label, list] of [[t('net.routesV4'), v4], [t('net.routesV6'), v6]]) {
    body += `<tr class="cron-section-row"><td colspan="${cols}">
      <div class="cron-section">
        <span class="cron-section-title">${esc(label)}</span>
        <span class="cron-section-sub text-dim">${list.length}</span>
      </div>
    </td></tr>`;
    if (!list.length) {
      body += `<tr><td colspan="${cols}" class="empty">${t('common.noData')}</td></tr>`;
      continue;
    }
    body += list.map(r => `
      <tr>
        <td class="mono">${esc(r.destination)}</td>
        <td class="mono">${esc(r.gateway)}</td>
        <td>${esc(r.flags)}</td>
        <td class="mono">${esc(r.interface)}</td>
        <td>${fmtExpire(r.expire)}</td>
      </tr>`).join('');
  }

  container.innerHTML = `<div class="card" style="padding:0;">
    <table>
      <thead><tr><th>${t('net.destination')}</th><th>${t('net.gateway')}</th><th>${t('common.status')}</th><th>${t('common.device')}</th><th>${t('net.expire')}</th></tr></thead>
      <tbody>${body}</tbody>
    </table>
  </div>`;
}

window.__fwpNetRefresh = () => {
  document.getElementById('net-interfaces').innerHTML =
    `<div class="card" style="padding:1rem;"><span class="spinner"></span> ${t('common.loading')}</div>`;
  document.getElementById('net-routes-container').innerHTML =
    `<div class="card" style="padding:0;"><table><tbody><tr><td colspan="5" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr></tbody></table></div>`;
  loadData();
};

window.__fwpNetDetail = (name) => {
  const iface = _interfaces.find(i => i.name === name);
  if (!iface) return;

  const flagsStr = iface.flags.join(', ');
  const ipv4Rows = iface.ipv4.map(ip => `
    <tr>
      <td class="mono">${esc(ip.address)}${ip.prefix_len != null ? `/${ip.prefix_len}` : ''}</td>
      <td class="mono">${esc(ip.netmask || '—')}</td>
      <td class="mono">${esc(ip.broadcast || '—')}</td>
      <td>${ip.is_alias ? t('net.alias') : '—'}</td>
    </tr>`).join('');
  const ipv6Rows = iface.ipv6.map(ip => `
    <tr>
      <td class="mono">${esc(ip.address)}${ip.prefix_len != null ? `/${ip.prefix_len}` : ''}</td>
      <td>${ip.is_alias ? t('net.alias') : '—'}</td>
    </tr>`).join('');

  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal" style="max-width:600px;">
      <h3>${esc(iface.name)} — ${t('net.interfaceInfo')}</h3>
      <div class="kv-grid">
        <div class="kv"><span class="kv-key">${t('common.status')}</span><span class="kv-val">${iface.is_up ? t('net.linkUp') : t('net.linkDown')} (${esc(iface.link_state)})</span></div>
        <div class="kv"><span class="kv-key">${t('net.flags')}</span><span class="kv-val mono">${esc(flagsStr)}</span></div>
        <div class="kv"><span class="kv-key">MAC</span><span class="kv-val mono">${esc(iface.mac || '—')}</span></div>
        <div class="kv"><span class="kv-key">MTU</span><span class="kv-val">${iface.mtu}</span></div>
        <div class="kv"><span class="kv-key">Metric</span><span class="kv-val">${iface.metric}</span></div>
        ${iface.groups.length ? `<div class="kv"><span class="kv-key">${t('net.groups')}</span><span class="kv-val">${iface.groups.map(g => `<span class="badge badge-dim">${esc(g)}</span>`).join(' ')}</span></div>` : ''}
      </div>
      ${ipv4Rows ? `
        <h4 style="margin-top:1rem;">IPv4</h4>
        <table>
          <thead><tr><th>${t('common.name')}</th><th>Netmask</th><th>Broadcast</th><th>${t('common.type')}</th></tr></thead>
          <tbody>${ipv4Rows}</tbody>
        </table>` : ''}
      ${ipv6Rows ? `
        <h4 style="margin-top:1rem;">IPv6</h4>
        <table>
          <thead><tr><th>${t('common.name')}</th><th>${t('common.type')}</th></tr></thead>
          <tbody>${ipv6Rows}</tbody>
        </table>` : ''}
      <div class="modal-actions">
        <button class="btn-secondary" data-act="close">${t('common.close')}</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay || e.target.dataset.act === 'close') overlay.remove();
  });
};

function fmtExpire(expire) {
  if (!expire) return '<span class="text-dim">\u2014</span>';
  const now = Math.floor(Date.now() / 1000);
  const remain = expire - now;
  if (remain <= 0) return '<span class="text-dim">\u2014</span>';
  const m = Math.floor(remain / 60);
  const s = remain % 60;
  return m > 0 ? `${m}m${s}s` : `${s}s`;
}

function fmtSpeed(baudrate) {
  if (!baudrate) return '—';
  const bps = baudrate;
  if (bps >= 1e9) return `${(bps / 1e9).toFixed(bps % 1e9 ? 1 : 0)} Gbps`;
  if (bps >= 1e6) return `${(bps / 1e6).toFixed(bps % 1e6 ? 0 : 0)} Mbps`;
  if (bps >= 1e3) return `${(bps / 1e3).toFixed(0)} Kbps`;
  return `${bps} bps`;
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
