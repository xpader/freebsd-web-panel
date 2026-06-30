// Cron — list / add / edit / delete / toggle crontab entries.
//
// Entries come from two source kinds, shown as separate sections:
//   • system  → /etc/crontab  (6-field format, editable "who" column)
//   • <user>  → /var/cron/tabs/<user>  (5-field format, run-as = username)
// Each section has its own "+ Add" button (creation target).

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { t } from '../i18n/index.js';

const SPECIALS = ['', '@reboot', '@yearly', '@annually', '@monthly', '@weekly', '@daily', '@midnight', '@hourly'];

let _all = [];

export async function renderCron(app) {
  renderLayout(app, '/cron', `
    <div class="page-header">
      <h1>${t('cron.title')}</h1>
      <p>${t('cron.subtitle')}</p>
      <p class="text-dim cron-note">${t('cron.backupNote')}</p>
    </div>
    <div class="toolbar">
      <input type="text" id="cron-filter" class="filter-input" placeholder="${t('cron.filter')}" oninput="window.__fwpCronFilter()" />
      <span id="cron-count" class="text-dim"></span>
      <div></div>
      <button onclick="window.__fwpCronAdd()">${t('cron.add')}</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr>
          <th>${t('cron.schedule')}</th>
          <th>${t('common.user')}</th>
          <th>${t('cron.command')}</th>
          <th>${t('cron.comment')}</th>
          <th>${t('common.status')}</th>
          <th>${t('common.actions')}</th>
        </tr></thead>
        <tbody id="cron-tbody">
          <tr><td colspan="6" class="empty"><span class="spinner"></span> ${t('common.loading')}</td></tr>
        </tbody>
      </table>
    </div>
  `);
  await load();
}

async function load() {
  const tbody = document.getElementById('cron-tbody');
  const countEl = document.getElementById('cron-count');
  try {
    _all = await api.get('/api/crontab');
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</td></tr>`;
    return;
  }
  if (countEl) countEl.textContent = t('cron.count', { n: _all.length });
  renderTable(_all);
}

// Source sections in display order: system first, then users alphabetically.
function orderedSources(entries) {
  const set = new Set(['system']);
  entries.forEach((e) => { if (e.source !== 'system') set.add(e.source); });
  return [...set].sort((a, b) => {
    if (a === 'system') return -1;
    if (b === 'system') return 1;
    return a.localeCompare(b);
  });
}

function sourceTitle(src) {
  return src === 'system' ? '/etc/crontab' : src;
}

function scheduleText(e) {
  if (e.kind === 'special') return e.special || '';
  return [e.minute, e.hour, e.dom, e.month, e.dow].filter((x) => x != null).join(' ');
}

function describe(e) {
  if (e.kind !== 'special' || !e.special) return '';
  return t('cron.alias_' + e.special.replace('@', ''));
}

function renderTable(list) {
  const tbody = document.getElementById('cron-tbody');
  if (!list.length) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">${t('cron.noEntries')}</td></tr>`;
    return;
  }
  const sources = orderedSources(list);
  let html = '';
  for (const src of sources) {
    const items = list.filter((e) => e.source === src);
    const matches = items.length;
    html += `<tr class="cron-section-row"><td colspan="6">
      <div class="cron-section">
        <span class="cron-section-title">${esc(sourceTitle(src))}</span>
        <span class="cron-section-sub text-dim">${t('cron.entriesCount', { n: matches })}</span>
        <button class="btn-secondary btn-sm" onclick="window.__fwpCronAdd('${escAttr(src)}')">${t('cron.addIn')}</button>
      </div>
    </td></tr>`;
    if (!matches) {
      html += `<tr><td colspan="6" class="empty">${t('cron.noEntriesInGroup')}</td></tr>`;
      continue;
    }
    for (const e of items) {
      const sched = esc(scheduleText(e));
      const desc = describe(e);
      const user = e.user ? esc(e.user) : `<span class="text-dim">—</span>`;
      const commentHtml = e.comment
        ? `<div class="cell-wrap cron-comment">${esc(e.comment)}</div>`
        : `<span class="text-dim">—</span>`;
      const statusBadge = e.disabled
        ? `<span class="badge badge-dim">${t('cron.disabled')}</span>`
        : `<span class="badge badge-success">${t('cron.enabled')}</span>`;
      const toggleLabel = e.disabled ? t('cron.enable') : t('cron.disable');
      const sysBadge = e.system_task
        ? `<span class="badge badge-warn" title="${escAttr(t('cron.systemTaskHint'))}">${t('cron.systemTask')}</span> `
        : '';
      html += `
        <tr${e.disabled ? ' class="row-dim"' : ''}>
          <td class="mono"><strong>${sched}</strong>${desc ? `<br><span class="text-dim">${esc(desc)}</span>` : ''}</td>
          <td class="mono">${user}</td>
          <td class="mono"><div class="cell-wrap">${sysBadge}${esc(e.command) || '<span class="text-dim">—</span>'}</div></td>
          <td>${commentHtml}</td>
          <td>${statusBadge}</td>
          <td>
            <button class="btn-secondary btn-sm" onclick="window.__fwpCronEdit('${escAttr(e.source)}', ${e.line})">${t('common.edit')}</button>
            <button class="btn-secondary btn-sm" onclick="window.__fwpCronToggle('${escAttr(e.source)}', ${e.line})">${toggleLabel}</button>
            <button class="btn-danger btn-sm" onclick="window.__fwpCronDel('${escAttr(e.source)}', ${e.line})">${t('common.delete')}</button>
          </td>
        </tr>`;
    }
  }
  tbody.innerHTML = html;
}

window.__fwpCronFilter = () => {
  const q = (document.getElementById('cron-filter')?.value || '').toLowerCase();
  if (!q) { renderTable(_all); return; }
  renderTable(_all.filter((e) =>
    scheduleText(e).toLowerCase().includes(q) ||
    (e.command || '').toLowerCase().includes(q) ||
    (e.user || '').toLowerCase().includes(q) ||
    (e.comment || '').toLowerCase().includes(q) ||
    (e.source || '').toLowerCase().includes(q)
  ));
};

// ---- custom modal ---------------------------------------------------------

// Cached selectable targets (system + users) so the dropdown is instant.
let _targets = null;
async function loadTargets() {
  if (_targets) return _targets;
  try {
    _targets = await api.get('/api/crontab/targets');
  } catch {
    _targets = [{ source: 'system', label: '/etc/crontab' }];
  }
  return _targets;
}

// `preselect` is the initially-selected target source (null → 'system').
// When editing, the target is locked (select disabled) since moving a task
// between files is not supported.
function entryModal(title, preselect, entry, submitLabel) {
  const isEdit = !!entry;
  const initialSource = preselect || (isEdit ? entry.source : 'system');
  const sp = isEdit && entry.kind === 'special' && entry.special ? entry.special : '';
  return new Promise(async (resolve) => {
    const targets = await loadTargets();
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    const opts = SPECIALS.map((s) => {
      if (s === '') return `<option value="" ${sp === '' ? 'selected' : ''}>${t('cron.custom')}</option>`;
      const desc = t('cron.alias_' + s.replace('@', ''));
      return `<option value="${s}" ${s === sp ? 'selected' : ''}>${s} — ${esc(desc)}</option>`;
    }).join('');
    const showFields = sp === '';

    const targetOpts = targets.map((tg) =>
      `<option value="${escAttr(tg.source)}" ${tg.source === initialSource ? 'selected' : ''}>${esc(tg.label)}</option>`
    ).join('');

    overlay.innerHTML = `
      <div class="modal">
        <h3>${esc(title)}</h3>
        <form id="cron-form">
          <div class="field">
            <label>${t('cron.target')}</label>
            <select id="cron-target" ${isEdit ? 'disabled' : ''}>${targetOpts}</select>
          </div>
          <div class="field">
            <label>${t('cron.scheduleType')}</label>
            <select id="cron-special">${opts}</select>
          </div>
          <div id="cron-fields" class="cron-fields" style="${showFields ? '' : 'display:none'}">
            <div class="field"><label>${t('cron.minute')}</label><input id="cron-minute" value="${escAttr(entry?.minute ?? '*')}" /></div>
            <div class="field"><label>${t('cron.hour')}</label><input id="cron-hour" value="${escAttr(entry?.hour ?? '*')}" /></div>
            <div class="field"><label>${t('cron.dom')}</label><input id="cron-dom" value="${escAttr(entry?.dom ?? '*')}" /></div>
            <div class="field"><label>${t('cron.month')}</label><input id="cron-month" value="${escAttr(entry?.month ?? '*')}" /></div>
            <div class="field"><label>${t('cron.dow')}</label><input id="cron-dow" value="${escAttr(entry?.dow ?? '*')}" /></div>
            <p class="cron-help text-dim">${t('cron.fieldsHelp')}</p>
          </div>
          <div id="cron-user-wrap" class="field">
            <label>${t('common.user')} <span style="color:var(--danger)">*</span></label>
            <input id="cron-user" value="${escAttr(entry?.user ?? 'root')}" placeholder="root" />
          </div>
          <div class="field">
            <label>${t('cron.command')} <span style="color:var(--danger)">*</span></label>
            <input id="cron-command" value="${escAttr(entry?.command ?? '')}" required placeholder="/usr/local/bin/backup.sh" />
          </div>
          <div class="field">
            <label>${t('cron.comment')}</label>
            <textarea id="cron-comment" rows="2" placeholder="${escAttr(t('cron.commentPlaceholder'))}">${esc(entry?.comment ?? '')}</textarea>
          </div>
          <div class="field cron-check">
            <label><input type="checkbox" id="cron-disabled" ${entry?.disabled ? 'checked' : ''} /> ${t('cron.disabledHint')}</label>
          </div>
          <div class="modal-actions">
            <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
            <button type="submit">${esc(submitLabel)}</button>
          </div>
        </form>
      </div>`;

    document.body.appendChild(overlay);

    // Show the run-as user field only for the system target (/etc/crontab).
    const syncUserField = () => {
      const isSystem = overlay.querySelector('#cron-target').value === 'system';
      overlay.querySelector('#cron-user-wrap').style.display = isSystem ? '' : 'none';
    };
    syncUserField();

    const close = (r) => { overlay.remove(); resolve(r); };
    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'cancel') close(null);
    });
    overlay.querySelector('#cron-target').addEventListener('change', syncUserField);
    overlay.querySelector('#cron-special').addEventListener('change', (e) => {
      overlay.querySelector('#cron-fields').style.display = e.target.value === '' ? '' : 'none';
    });
    overlay.querySelector('#cron-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const source = isEdit ? entry.source : overlay.querySelector('#cron-target').value;
      const isSystem = source === 'system';
      const special = overlay.querySelector('#cron-special').value;
      const custom = special === '';
      const result = {
        source,
        special: custom ? null : special,
        minute: custom ? overlay.querySelector('#cron-minute').value.trim() : null,
        hour: custom ? overlay.querySelector('#cron-hour').value.trim() : null,
        dom: custom ? overlay.querySelector('#cron-dom').value.trim() : null,
        month: custom ? overlay.querySelector('#cron-month').value.trim() : null,
        dow: custom ? overlay.querySelector('#cron-dow').value.trim() : null,
        user: isSystem ? overlay.querySelector('#cron-user').value.trim() : null,
        command: overlay.querySelector('#cron-command').value,
        comment: overlay.querySelector('#cron-comment').value,
        disabled: overlay.querySelector('#cron-disabled').checked,
      };
      close(result);
    });

    setTimeout(() => {
      const focus = isEdit ? overlay.querySelector('#cron-command') : overlay.querySelector('#cron-special');
      if (focus) focus.focus();
    }, 50);
  });
}

window.__fwpCronAdd = async (preselect) => {
  const result = await entryModal(t('cron.addTitle'), preselect || null, null, t('cron.add'));
  if (!result) return;
  const { source, ...entry } = result;
  api.post('/api/crontab', { source, ...entry }).then(() => {
    toast(t('cron.added'));
    load();
  }).catch((e) => toast(e.message || t('common.saveFailed', { msg: '' }), 'error'));
};

window.__fwpCronEdit = async (source, line) => {
  const entry = _all.find((e) => e.source === source && e.line === line);
  if (!entry) return;
  const result = await entryModal(t('cron.editTitle'), source, entry, t('common.save'));
  if (!result) return;
  api.put('/api/crontab', { source, line, ...result }).then(() => {
    toast(t('cron.saved'));
    load();
  }).catch((e) => toast(e.message || t('common.saveFailed', { msg: '' }), 'error'));
};

window.__fwpCronToggle = async (source, line) => {
  const entry = _all.find((e) => e.source === source && e.line === line);
  if (!entry) return;
  const payload = {
    source, line,
    special: entry.special || null,
    minute: entry.minute || null,
    hour: entry.hour || null,
    dom: entry.dom || null,
    month: entry.month || null,
    dow: entry.dow || null,
    user: entry.user || null,
    command: entry.command,
    comment: entry.comment,
    disabled: !entry.disabled,
  };
  api.put('/api/crontab', payload).then(() => {
    toast(entry.disabled ? t('cron.enabled') : t('cron.disabled'));
    load();
  }).catch((e) => toast(e.message || t('common.saveFailed', { msg: '' }), 'error'));
};

window.__fwpCronDel = async (source, line) => {
  const entry = _all.find((e) => e.source === source && e.line === line);
  if (!entry) return;
  if (!await confirmDialog(t('cron.deleteTitle'), t('cron.deleteConfirm', { sched: scheduleText(entry) }))) return;
  api.del(`/api/crontab?source=${encodeURIComponent(source)}&line=${line}`).then(() => {
    toast(t('cron.deleted'));
    load();
  }).catch((e) => toast(e.message || t('common.deleteFailed'), 'error'));
};

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

function escAttr(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}
