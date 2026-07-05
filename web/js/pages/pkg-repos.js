// pkg repository management — grouped by file, add/edit/delete repos.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { alertDialog } from '../ui/alertDialog.js';
import { confirmDialog } from '../ui/confirm.js';
import { t } from '../i18n/index.js';

// Preset templates for quick fill.
const PRESETS = [
  { key: 'repoPresetOfficialLatest', name: 'FreeBSD-ports', filename: 'FreeBSD.conf', url: 'pkg+https://pkg.freebsd.org/${ABI}/latest', mirror_type: 'srv', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg' },
  { key: 'repoPresetOfficialQuarterly', name: 'FreeBSD-ports', filename: 'FreeBSD.conf', url: 'pkg+https://pkg.freebsd.org/${ABI}/quarterly', mirror_type: 'srv', signature_type: 'fingerprints', fingerprints: '/usr/share/keys/pkg' },
  { key: 'repoPresetUstc', name: 'ustc', filename: 'ustc.conf', url: 'https://mirrors.ustc.edu.cn/freebsd-pkg/${ABI}/quarterly', mirror_type: 'none', signature_type: 'none' },
  { key: 'repoPresetTuna', name: 'tuna', filename: 'tuna.conf', url: 'https://mirrors.tuna.tsinghua.edu.cn/freebsd-pkg/${ABI}/quarterly', mirror_type: 'none', signature_type: 'none' },
];

let _repoFiles = [];

export async function renderPkgRepos(app) {
  renderLayout(app, '/pkg/repos', `
    <div class="page-header">
      <h1>${t('nav.pkgRepos')}</h1>
      <p>${t('pkg.repoSubtitle')}</p>
    </div>
    <div class="toolbar">
      <span id="repo-count" class="text-dim"></span>
      <div class="flex">
        <button onclick="window.__fwpRepoAdd()"><i class="fa-solid fa-plus"></i> ${t('pkg.repoAdd')}</button>
        <button onclick="window.__fwpRepoRefresh()"><i class="fa-solid fa-rotate-right"></i> ${t('pkg.repoRefresh')}</button>
      </div>
    </div>
    <div id="repo-files-body">
      <div class="empty"><span class="spinner"></span> ${t('common.loading')}</div>
    </div>
  `);

  await loadRepos();
}

async function loadRepos() {
  const el = document.getElementById('repo-files-body');
  const countEl = document.getElementById('repo-count');
  try {
    _repoFiles = await api.get('/api/pkg/repos');
    const totalRepos = _repoFiles.reduce((sum, f) => sum + f.repos.length, 0);
    if (countEl) countEl.textContent = t('pkg.repoFileCount', { n: _repoFiles.length, m: totalRepos });

    if (!_repoFiles.length) {
      el.innerHTML = `<div class="empty">${t('pkg.repoNoRepos')}</div>`;
      return;
    }

    el.innerHTML = _repoFiles.map((file) => renderFileSection(file)).join('');
  } catch (e) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(e.message || '') })}</div>`;
  }
}

function renderFileSection(file) {
  const sourceBadge = file.is_system
    ? `<span class="badge badge-dim">${t('pkg.repoSystem')}</span>`
    : `<span class="badge badge-success">${t('pkg.repoCustom')}</span>`;

  return `
    <div class="card" style="padding:0; margin-bottom:16px;">
      <div style="padding:12px 16px; display:flex; align-items:center; gap:8px; border-bottom:1px solid var(--border);">
        <i class="fa-solid fa-file-lines"></i>
        <strong>${esc(file.filename)}</strong>
        ${sourceBadge}
        <span class="text-dim" style="font-size:12px; flex:1;">${esc(file.path)}</span>
        <button class="btn-secondary btn-sm" onclick="window.__fwpRepoAddToFile('${escAttr(file.filename)}')"><i class="fa-solid fa-plus"></i> ${t('pkg.repoAdd')}</button>
      </div>
      ${file.repos.length ? `
        <table>
          <thead><tr>
            <th>${t('pkg.repoName')}</th>
            <th>${t('pkg.repoUrl')}</th>
            <th>${t('pkg.repoEnabled')}</th>
            <th>${t('pkg.repoPriority')}</th>
            <th>${t('common.actions')}</th>
          </tr></thead>
          <tbody>
            ${file.repos.map((r) => renderRepoRow(r, file)).join('')}
          </tbody>
        </table>
      ` : `<div class="empty">${t('pkg.repoNoRepos')}</div>`}
    </div>
  `;
}

function renderRepoRow(r, file) {
  const originBadge = r.is_system_origin
    ? ` <span class="badge badge-dim">${t('pkg.repoSystem')}</span>`
    : '';
  // Can delete: only user-origin repos in non-system files
  const canDelete = !r.is_system_origin;
  return `
    <tr>
      <td class="mono"><strong>${esc(r.name)}</strong>${originBadge}</td>
      <td class="mono text-dim" style="max-width:360px; overflow:hidden; text-overflow:ellipsis;">${esc(r.url)}</td>
      <td>${r.enabled
        ? `<span class="badge badge-success">${t('pkg.repoEnabledYes')}</span>`
        : `<span class="badge badge-dim">${t('pkg.repoEnabledNo')}</span>`}
      </td>
      <td class="mono">${esc(r.priority || 0)}</td>
      <td>
        <div class="btn-group">
          <button class="btn-secondary btn-sm" onclick="window.__fwpRepoToggle('${escAttr(file.path)}', '${escAttr(r.name)}', ${!r.enabled})">${r.enabled ? t('common.disable') : t('common.enable')}</button>
          <button class="btn-secondary btn-sm" onclick="window.__fwpRepoEdit('${escAttr(file.path)}', '${escAttr(r.name)}')">${t('common.edit')}</button>
          ${canDelete ? `<button class="btn-danger btn-sm" onclick="window.__fwpRepoDelete('${escAttr(file.path)}', '${escAttr(r.name)}')">${t('common.delete')}</button>` : ''}
        </div>
      </td>
    </tr>
  `;
}

// ---- Toggle enable/disable ----

window.__fwpRepoToggle = async (filePath, name, enable) => {
  // Find the repo to get its full config.
  for (const file of _repoFiles) {
    const repo = file.repos.find((r) => r.name === name && file.path === filePath);
    if (!repo) continue;
    try {
      await api.put(`/api/pkg/repos/${encodeURIComponent(name)}`, {
        file: filePath,
        url: repo.url,
        enabled: enable,
        mirror_type: repo.mirror_type,
        signature_type: repo.signature_type,
        fingerprints: repo.fingerprints,
        pubkey: repo.pubkey,
        priority: repo.priority,
        ip_version: repo.ip_version,
      });
      toast(t('pkg.repoUpdateOk', { name }), 'success');
      loadRepos();
    } catch (e) {
      await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
    return;
  }
};

// ---- Delete ----

window.__fwpRepoDelete = async (filePath, name) => {
  // Check if the repo is a system-origin repo.
  for (const file of _repoFiles) {
    const repo = file.repos.find((r) => r.name === name && file.path === filePath);
    if (repo && repo.is_system_origin) {
      await alertDialog(t('pkg.repoSystemNoDelete'), t('pkg.repoSystemNoDelete'));
      return;
    }
  }
  const ok = await confirmDialog(t('pkg.repoDelete'), t('pkg.repoDeleteConfirm', { name }));
  if (!ok) return;
  try {
    await api.del(`/api/pkg/repos/${encodeURIComponent(name)}?file=${encodeURIComponent(filePath)}`);
    toast(t('pkg.repoDeleteOk', { name }), 'success');
    loadRepos();
  } catch (e) {
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
  }
};

// ---- Edit ----

window.__fwpRepoEdit = async (filePath, name) => {
  for (const file of _repoFiles) {
    const repo = file.repos.find((r) => r.name === name && file.path === filePath);
    if (repo) {
      showRepoModal({ repo, file: file });
      return;
    }
  }
};

// ---- Add ----

window.__fwpRepoAdd = () => {
  showRepoModal(null);
};

window.__fwpRepoAddToFile = (filename) => {
  showRepoModal(null, filename);
};

function showRepoModal(existing, presetFilename) {
  const isEdit = !!existing;
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';

  const r = isEdit ? existing.repo : { name: '', url: '', enabled: true, mirror_type: 'NONE', signature_type: 'NONE', fingerprints: null, pubkey: null, priority: 0, ip_version: 0 };
  const filePath = isEdit ? existing.file.path : '';

  // Build file selector options from existing user files.
  const userFiles = _repoFiles.filter((f) => !f.is_system);
  const fileOpts = userFiles.map((f) => `<option value="${escAttr(f.filename)}">${esc(f.filename)}</option>`).join('');

  overlay.innerHTML = `
    <div class="modal" style="max-width:600px;">
      <h3>${isEdit ? t('pkg.repoEdit') : t('pkg.repoAdd')}</h3>

      ${!isEdit ? `
        <div style="margin-bottom:16px;">
          <div class="card-title" style="font-size:13px;">${t('pkg.repoPresetTitle')}</div>
          <div class="flex" style="flex-wrap:wrap; gap:6px; margin-top:6px;" id="repo-presets">
            ${PRESETS.map((p, i) => `<button class="btn-secondary btn-sm" data-preset="${i}">${t('pkg.' + p.key)}</button>`).join('')}
          </div>
        </div>
      ` : ''}

      <div class="repo-form">
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoTargetFile')}</label>
          <div class="repo-form-field">
            ${isEdit
              ? `<span class="mono">${esc(existing.file.filename)}</span>`
              : `<select id="repo-fld-filesel" class="filter-input" style="width:auto;">
                  <option value="">— ${t('pkg.repoNewFile')} —</option>
                  ${fileOpts}
                </select>
                <input type="text" id="repo-fld-filename" class="filter-input" value="" placeholder="e.g. FreeBSD.conf" style="width:180px; margin-left:6px;" />`
            }
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoName')}</label>
          <div class="repo-form-field">
            <input type="text" id="repo-fld-name" class="filter-input" value="${escAttr(r.name)}" placeholder="${t('pkg.repoNamePh')}" style="width:100%;" ${(isEdit && r.is_system_origin) ? 'readonly' : ''} />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoUrl')}</label>
          <div class="repo-form-field">
            <input type="text" id="repo-fld-url" class="filter-input" value="${escAttr(r.url)}" placeholder="${t('pkg.repoUrlPh')}" style="width:100%;" />
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoEnabled')}</label>
          <div class="repo-form-field">
            <label style="display:flex;align-items:center;gap:6px;cursor:pointer;"><input type="checkbox" id="repo-fld-enabled" ${r.enabled ? 'checked' : ''} /> ${t('pkg.repoEnabledYes')}</label>
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoMirrorType')}</label>
          <div class="repo-form-field">
            <select id="repo-fld-mirror" class="filter-input" style="width:auto;">
              <option value="NONE" ${r.mirror_type === 'NONE' ? 'selected' : ''}>NONE</option>
              <option value="SRV" ${r.mirror_type === 'SRV' ? 'selected' : ''}>SRV</option>
              <option value="HTTP" ${r.mirror_type === 'HTTP' ? 'selected' : ''}>HTTP</option>
            </select>
            <p class="repo-form-hint">${t('pkg.repoMirrorHint')}</p>
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoSignatureType')}</label>
          <div class="repo-form-field">
            <select id="repo-fld-sigtype" class="filter-input" style="width:auto;">
              <option value="NONE" ${r.signature_type === 'NONE' ? 'selected' : ''}>NONE</option>
              <option value="FINGERPRINTS" ${r.signature_type === 'FINGERPRINTS' ? 'selected' : ''}>FINGERPRINTS</option>
              <option value="PUBKEY" ${r.signature_type === 'PUBKEY' ? 'selected' : ''}>PUBKEY</option>
            </select>
            <p class="repo-form-hint">${t('pkg.repoSignatureHint')}</p>
          </div>
        </div>
        <div class="repo-form-row" id="repo-row-fingerprints" style="display:none;">
          <label class="repo-form-label">${t('pkg.repoFingerprints')}</label>
          <div class="repo-form-field">
            <input type="text" id="repo-fld-fingerprints" class="filter-input" value="${escAttr(r.fingerprints || '')}" placeholder="/usr/share/keys/pkg" style="width:100%;" />
            <p class="repo-form-hint">${t('pkg.repoFingerprintsHint')}</p>
          </div>
        </div>
        <div class="repo-form-row" id="repo-row-pubkey" style="display:none;">
          <label class="repo-form-label">${t('pkg.repoPubkey')}</label>
          <div class="repo-form-field">
            <input type="text" id="repo-fld-pubkey" class="filter-input" value="${escAttr(r.pubkey || '')}" placeholder="/usr/share/keys/pubkey.pem" style="width:100%;" />
            <p class="repo-form-hint">${t('pkg.repoPubkeyHint')}</p>
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoPriority')}</label>
          <div class="repo-form-field">
            <input type="number" id="repo-fld-priority" class="filter-input" value="${esc(r.priority || 0)}" style="width:80px;" />
            <p class="repo-form-hint">${t('pkg.repoPriorityHint')}</p>
          </div>
        </div>
        <div class="repo-form-row">
          <label class="repo-form-label">${t('pkg.repoIpVersion')}</label>
          <div class="repo-form-field">
            <select id="repo-fld-ipver" class="filter-input" style="width:auto;">
              <option value="0" ${r.ip_version === 0 ? 'selected' : ''}>${t('common.default')}</option>
              <option value="4" ${r.ip_version === 4 ? 'selected' : ''}>IPv4</option>
              <option value="6" ${r.ip_version === 6 ? 'selected' : ''}>IPv6</option>
            </select>
          </div>
        </div>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
        <button data-act="save">${t('common.save')}</button>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);

  // File selector behavior: show/hide filename input.
  if (!isEdit) {
    const filesel = document.getElementById('repo-fld-filesel');
    const filenameInput = document.getElementById('repo-fld-filename');

    // Pre-select a filename if provided via quick-add button.
    if (presetFilename) {
      const isExistingUser = userFiles.some((f) => f.filename === presetFilename);
      if (isExistingUser) {
        filesel.value = presetFilename;
        filenameInput.style.display = 'none';
      } else {
        // System file or brand-new → fill filename input, hide dropdown.
        filesel.style.display = 'none';
        filenameInput.value = presetFilename;
        filenameInput.style.display = '';
      }
    } else {
      // Default: dropdown shows "— New file —" (value=""), so filename input should be visible.
      if (!fileOpts) {
        filesel.style.display = 'none';
      }
      filenameInput.style.display = filesel.value ? 'none' : '';
    }

    filesel.addEventListener('change', () => {
      filenameInput.style.display = filesel.value ? 'none' : '';
    });
  }

  const sigTypeSel = document.getElementById('repo-fld-sigtype');
  const fpRow = document.getElementById('repo-row-fingerprints');
  const pkRow = document.getElementById('repo-row-pubkey');

  function updateSigRows() {
    const v = sigTypeSel.value;
    if (fpRow) fpRow.style.display = v === 'FINGERPRINTS' ? '' : 'none';
    if (pkRow) pkRow.style.display = v === 'PUBKEY' ? '' : 'none';
  }
  sigTypeSel.addEventListener('change', updateSigRows);
  updateSigRows();

  // Preset buttons.
  const presetContainer = document.getElementById('repo-presets');
  if (presetContainer) {
    presetContainer.addEventListener('click', (ev) => {
      const btn = ev.target.closest('[data-preset]');
      if (!btn) return;
      const p = PRESETS[parseInt(btn.dataset.preset)];
      const nameInput = document.getElementById('repo-fld-name');
      const urlInput = document.getElementById('repo-fld-url');
      const mirrorSel = document.getElementById('repo-fld-mirror');
      const sigSel = document.getElementById('repo-fld-sigtype');
      const fpInput = document.getElementById('repo-fld-fingerprints');
      if (nameInput) nameInput.value = p.name;
      if (urlInput) urlInput.value = p.url;
      if (mirrorSel) mirrorSel.value = (p.mirror_type || 'none').toUpperCase();
      if (sigSel) {
        sigSel.value = (p.signature_type || 'none').toUpperCase();
        updateSigRows();
      }
      if (fpInput) fpInput.value = p.fingerprints || '';
      // Auto-fill filename: switch to "new file" mode and set the filename.
      const filesel = document.getElementById('repo-fld-filesel');
      const filenameInput = document.getElementById('repo-fld-filename');
      if (filesel) {
        filesel.value = '';
        filesel.style.display = 'none';
      }
      if (filenameInput) {
        filenameInput.value = p.filename;
        filenameInput.style.display = '';
      }
    });
  }

  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) overlay.remove();
    if (e.target.dataset.act === 'cancel') overlay.remove();
  });

  overlay.querySelector('[data-act="save"]').addEventListener('click', async () => {
    const repoBody = {
      name: document.getElementById('repo-fld-name').value.trim(),
      url: document.getElementById('repo-fld-url').value.trim(),
      enabled: document.getElementById('repo-fld-enabled').checked,
      mirror_type: document.getElementById('repo-fld-mirror').value,
      signature_type: document.getElementById('repo-fld-sigtype').value,
      fingerprints: document.getElementById('repo-fld-fingerprints').value.trim() || null,
      pubkey: document.getElementById('repo-fld-pubkey').value.trim() || null,
      priority: parseInt(document.getElementById('repo-fld-priority').value) || 0,
      ip_version: parseInt(document.getElementById('repo-fld-ipver').value) || 0,
    };

    if (!repoBody.name || !repoBody.url) {
      await alertDialog(t('common.fillRequired'), t('common.fillRequired'));
      return;
    }

    try {
      if (isEdit) {
        await api.put(`/api/pkg/repos/${encodeURIComponent(repoBody.name)}`, {
          file: filePath,
          url: repoBody.url,
          enabled: repoBody.enabled,
          mirror_type: repoBody.mirror_type,
          signature_type: repoBody.signature_type,
          fingerprints: repoBody.fingerprints,
          pubkey: repoBody.pubkey,
          priority: repoBody.priority,
          ip_version: repoBody.ip_version,
        });
        toast(t('pkg.repoUpdateOk', { name: repoBody.name }), 'success');
      } else {
        // Determine target filename.
        let filename;
        const filesel = document.getElementById('repo-fld-filesel');
        if (filesel && filesel.value) {
          filename = filesel.value;
        } else {
          filename = document.getElementById('repo-fld-filename').value.trim();
        }
        if (!filename) {
          await alertDialog(t('pkg.repoFileRequired'), t('pkg.repoFileRequired'));
          return;
        }
        await api.post('/api/pkg/repos', { filename, ...repoBody });
        toast(t('pkg.repoCreateOk', { name: repoBody.name }), 'success');
      }
      overlay.remove();
      loadRepos();
    } catch (e) {
      await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
    }
  });
}

// ---- Refresh catalog (pkg update -f) ----

window.__fwpRepoRefresh = async () => {
  let taskId;
  try {
    const res = await api.post('/api/pkg/repos/update', {});
    taskId = res.task_id;
  } catch (e) {
    await alertDialog(t('common.operationFailed'), e.message || t('common.operationFailed'));
    return;
  }
  showRefreshModal(taskId);
};

function showRefreshModal(taskId) {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal" style="max-width:680px;">
      <h3 id="repo-refresh-title">
        <span class="spinner"></span>
        ${t('pkg.repoRefreshing')}
      </h3>
      <div id="repo-refresh-output" style="max-height:400px; overflow-y:auto; background:var(--bg); border:1px solid var(--border); border-radius:var(--radius); padding:12px; margin-bottom:12px; font-family:monospace; font-size:12px; white-space:pre-wrap; word-break:break-all;"></div>
      <div class="modal-actions">
        <button id="repo-refresh-close" class="btn-secondary" disabled>${t('common.close')}</button>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);

  const outputEl = document.getElementById('repo-refresh-output');
  const closeBtn = document.getElementById('repo-refresh-close');
  const titleEl = document.getElementById('repo-refresh-title');
  closeBtn.addEventListener('click', () => overlay.remove());

  const token = sessionStorage.getItem('fwp_token');
  const url = `/api/pkg/tasks/${encodeURIComponent(taskId)}/stream?token=${encodeURIComponent(token)}`;
  const es = new EventSource(url);

  const finish = async (success) => {
    es.close();
    closeBtn.disabled = false;
    const label = success ? t('pkg.repoRefreshDone') : t('pkg.repoRefreshFailed');
    const color = success ? 'var(--success)' : 'var(--danger)';
    titleEl.innerHTML = `<span style="color:${color}; font-weight:700;">${esc(label)}</span>`;
    if (success) {
      toast(label);
    } else {
      await alertDialog(t('pkg.repoRefreshFailed'), label);
    }
  };

  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data.lines && data.lines.length) {
        const atBottom = outputEl.scrollTop + outputEl.clientHeight >= outputEl.scrollHeight - 2;
        outputEl.textContent += data.lines.join('\n') + '\n';
        if (atBottom) outputEl.scrollTop = outputEl.scrollHeight;
      }
      if (data.status && data.status !== 'running') {
        finish(data.status === 'done');
      }
    } catch {}
  };

  es.addEventListener('done', () => {
    es.close();
    closeBtn.disabled = false;
  });

  es.onerror = () => {
    es.close();
    api.get(`/api/pkg/tasks/${encodeURIComponent(taskId)}`).then((task) => {
      if (task.status !== 'running') {
        finish(task.status === 'done');
      } else {
        closeBtn.disabled = false;
      }
    }).catch(() => {
      closeBtn.disabled = false;
    });
  };
}

// ---- Utilities ----

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
