// File manager — directory tree (left) + file listing (right) with
// list/grid views, upload, download, rename, delete, properties.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { t } from '../i18n/index.js';

const START_DIR = '/root';
const ROOT = '/';

// In-memory view state (per page render).
let currentDir = START_DIR;
let viewMode = localStorage.getItem('fwp_fm_view') || 'list'; // 'list' | 'grid'
let expanded = new Set([ROOT]);                 // expanded tree paths
let treeChildren = new Map();                    // path -> DirEntry[] (dirs only)
let lastEntries = [];                            // last loaded listing (for re-render)

function token() {
  return sessionStorage.getItem('fwp_token');
}

// Icon by type + extension. Folders get a folder glyph, symlinks an arrow,
// files get an extension-based icon with a sensible default.
function fileIcon(e) {
  if (e.is_dir) return '<i class="fa-solid fa-folder"></i>';
  if (e.is_symlink) return '<i class="fa-solid fa-link"></i>';
  const ext = (e.name.split('.').pop() || '').toLowerCase();
  const map = {
    txt: 'fa-regular fa-file-lines', log: 'fa-regular fa-file-lines',
    md: 'fa-regular fa-file-lines',
    png: 'fa-regular fa-file-image', jpg: 'fa-regular fa-file-image', jpeg: 'fa-regular fa-file-image',
    gif: 'fa-regular fa-file-image', webp: 'fa-regular fa-file-image', svg: 'fa-regular fa-file-image', bmp: 'fa-regular fa-file-image',
    mp4: 'fa-regular fa-file-video', mkv: 'fa-regular fa-file-video', avi: 'fa-regular fa-file-video', mov: 'fa-regular fa-file-video', webm: 'fa-regular fa-file-video',
    mp3: 'fa-regular fa-file-audio', wav: 'fa-regular fa-file-audio', flac: 'fa-regular fa-file-audio', ogg: 'fa-regular fa-file-audio',
    zip: 'fa-regular fa-file-zipper', gz: 'fa-regular fa-file-zipper', tar: 'fa-regular fa-file-zipper', xz: 'fa-regular fa-file-zipper', bz2: 'fa-regular fa-file-zipper', '7z': 'fa-regular fa-file-zipper', zst: 'fa-regular fa-file-zipper',
    iso: 'fa-regular fa-file-zipper',
    pdf: 'fa-regular fa-file-pdf',
    sh: 'fa-regular fa-file-code', py: 'fa-regular fa-file-code', pl: 'fa-regular fa-file-code', rb: 'fa-regular fa-file-code', js: 'fa-regular fa-file-code', rs: 'fa-regular fa-file-code', c: 'fa-regular fa-file-code', h: 'fa-regular fa-file-code', json: 'fa-regular fa-file-code', conf: 'fa-regular fa-file-code',
  };
  const cls = map[ext] || 'fa-regular fa-file';
  return `<i class="${cls}"></i>`;
}

// Folder icon for the tree: root vs nested.
function treeIcon(path) {
  return path === ROOT
    ? '<i class="fa-solid fa-folder-tree"></i>'
    : '<i class="fa-solid fa-folder"></i>';
}

export async function renderFiles(app) {
  renderLayout(app, '/filesystem/files', `
    <div class="page-header">
      <h1>${t('fm.title')}</h1>
      <p>${t('fm.subtitle')}</p>
    </div>
    <div class="fm-wrap">
      <div class="fm-tree">
        <div class="fm-tree-head">${t('fm.treeHead')}</div>
        <div id="fm-tree-body"><span class="spinner"></span></div>
      </div>
      <div class="fm-main" id="fm-main"><div class="empty"><span class="spinner"></span> ${t('common.loading')}</div></div>
    </div>
    <input type="file" id="fm-upload-input" multiple style="display:none" />
  `);

  document.getElementById('fm-upload-input').addEventListener('change', onUploadPicked);

  try {
    await initTree();
    await openDir(currentDir);
  } catch (err) {
    document.getElementById('fm-main').innerHTML =
      `<div class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
  }
}

// ===== Tree (left pane) =====

async function initTree() {
  if (!treeChildren.has(ROOT)) {
    treeChildren.set(ROOT, await fetchDirs(ROOT));
  }
  await ensureAncestors(currentDir);
  renderTree();
}

async function fetchDirs(path) {
  const entries = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
  return entries.filter((e) => e.is_dir);
}

/// Load + expand every ancestor of `path` so it is visible in the tree.
async function ensureAncestors(path) {
  const parts = path.split('/').filter(Boolean);
  let cur = '';
  for (const part of parts) {
    cur = cur + '/' + part;
    if (!treeChildren.has(cur)) {
      try {
        treeChildren.set(cur, await fetchDirs(cur));
      } catch {
        treeChildren.set(cur, []);
      }
    }
    expanded.add(cur);
  }
  expanded.add(ROOT);
}

function renderTree() {
  const el = document.getElementById('fm-tree-body');
  if (!el) return;
  el.innerHTML = treeNodeHtml(ROOT);
  el.querySelectorAll('[data-toggle]').forEach((node) => {
    node.addEventListener('click', (ev) => {
      ev.stopPropagation();
      toggleExpand(node.getAttribute('data-toggle'));
    });
  });
  el.querySelectorAll('[data-dir]').forEach((node) => {
    node.addEventListener('click', () => openDir(node.getAttribute('data-dir')));
  });
}

function treeNodeHtml(path) {
  const depth = pathDepth(path);
  const name = path === ROOT ? '/' : path.split('/').filter(Boolean).pop();
  const isExpanded = expanded.has(path);
  const loaded = treeChildren.get(path);
  const hasLoaded = loaded !== undefined;
  const arrow = !hasLoaded ? '<i class="fa-solid fa-caret-right"></i>' : loaded.length > 0 ? (isExpanded ? '<i class="fa-solid fa-caret-down"></i>' : '<i class="fa-solid fa-caret-right"></i>') : '';
  const children = isExpanded && hasLoaded ? loaded.map((d) => treeNodeHtml(d.path)).join('') : '';
  return `
    <div class="fm-tree-node">
      <div class="fm-tree-row ${path === currentDir ? 'active' : ''}" data-dir="${esc(path)}" style="padding-left:${depth * 14 + 6}px">
        <span class="fm-tree-arrow" data-toggle="${esc(path)}">${arrow}</span>
        <span class="fm-tree-name">
          <span class="fm-tree-ico">${treeIcon(path)}</span>${esc(name)}
        </span>
      </div>
      <div class="fm-tree-children">${children}</div>
    </div>`;
}

async function toggleExpand(path) {
  if (!treeChildren.has(path)) {
    try {
      treeChildren.set(path, await fetchDirs(path));
    } catch {
      treeChildren.set(path, []);
    }
  }
  if (expanded.has(path)) expanded.delete(path);
  else expanded.add(path);
  renderTree();
}

// ===== Listing (right pane) =====

async function openDir(path) {
  currentDir = path;
  if (path !== ROOT) await ensureAncestors(path);
  renderTree();
  await loadListing(path);
}

async function loadListing(path) {
  const el = document.getElementById('fm-main');
  el.innerHTML = '<div class="empty"><span class="spinner"></span> ' + t('common.loading') + '</div>';
  try {
    lastEntries = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
    renderListing(path, lastEntries);
  } catch (err) {
    el.innerHTML = `<div class="empty">${t('common.loadFailed', { msg: esc(err.message || '') })}</div>`;
  }
}

function renderListing(path, entries) {
  const el = document.getElementById('fm-main');
  el.innerHTML = `
    <div class="fm-toolbar">
      <div class="fm-breadcrumb" id="fm-breadcrumb">${breadcrumbHtml(path)}</div>
      <div class="fm-actions">
        <button class="btn-secondary btn-sm" id="fm-upload-btn"><i class="fa-solid fa-upload"></i> ${t('fm.upload')}</button>
        <button class="btn-secondary btn-sm" id="fm-mkdir-btn"><i class="fa-solid fa-folder-plus"></i> ${t('fm.mkdir')}</button>
        <div class="fm-view-toggle">
          <button class="btn-secondary btn-sm ${viewMode === 'list' ? 'active-range' : ''}" data-view="list"><i class="fa-solid fa-list"></i> ${t('fm.listView')}</button>
          <button class="btn-secondary btn-sm ${viewMode === 'grid' ? 'active-range' : ''}" data-view="grid"><i class="fa-solid fa-table-cells"></i> ${t('fm.gridView')}</button>
        </div>
      </div>
    </div>
    <div class="fm-listing">${viewMode === 'list' ? listHtml(entries) : gridHtml(entries)}</div>`;

  bindListingEvents(el);
}

// Bindings are scoped to the main pane so the directory tree's own `[data-dir]`
// rows (which also navigate) aren't double-bound here.
function bindListingEvents(el) {
  el.querySelector('#fm-upload-btn').addEventListener('click', () => {
    document.getElementById('fm-upload-input').click();
  });
  el.querySelector('#fm-mkdir-btn').addEventListener('click', onMkdir);
  el.querySelectorAll('[data-view]').forEach((btn) => {
    btn.addEventListener('click', () => {
      viewMode = btn.getAttribute('data-view');
      localStorage.setItem('fwp_fm_view', viewMode);
      renderListing(currentDir, lastEntries);
    });
  });
  el.querySelectorAll('[data-dir]').forEach((node) => {
    node.addEventListener('click', () => openDir(node.getAttribute('data-dir')));
  });
  el.querySelectorAll('[data-act]').forEach((btn) => {
    btn.addEventListener('click', (ev) => {
      ev.stopPropagation();
      const act = btn.getAttribute('data-act');
      const p = btn.getAttribute('data-path');
      const isDir = btn.getAttribute('data-dir-flag') === '1';
      if (act === 'download') downloadFile(p);
      else if (act === 'rename') onRename(p);
      else if (act === 'delete') onDelete(p, isDir);
      else if (act === 'stat') onStat(p);
    });
  });
}

function breadcrumbHtml(path) {
  let crumbs = `<a class="fm-crumb" data-dir="${ROOT}">/</a>`;
  let cur = '';
  const parts = path.split('/').filter(Boolean);
  for (let i = 0; i < parts.length; i++) {
    cur = cur + '/' + parts[i];
    // The root "/" crumb already supplies the leading slash; only put a
    // separator between non-root segments to avoid "//root".
    if (i > 0) crumbs += `<span class="fm-sep">/</span>`;
    crumbs += `<a class="fm-crumb" data-dir="${esc(cur)}">${esc(parts[i])}</a>`;
  }
  return crumbs;
}

function listHtml(entries) {
  if (!entries.length) return '<div class="empty">' + t('fm.emptyDir') + '</div>';
  const rows = entries.map((e) => {
    const icon = fileIcon(e);
    const dl = e.is_dir ? '' : `<button class="fm-act" data-act="download" data-path="${esc(e.path)}" title="${t('fm.download')}"><i class="fa-solid fa-download"></i></button>`;
    return `
      <tr>
        <td class="fm-name-cell">
          <div class="fm-cell-flex">
            <span class="fm-row-ico">${icon}</span>
            ${e.is_dir
              ? `<a class="fm-name-link" data-dir="${esc(e.path)}">${esc(e.name)}</a>`
              : `<span class="fm-name">${esc(e.name)}</span>`}
          </div>
        </td>
        <td class="mono">${e.is_dir ? '—' : fmtBytes(e.size)}</td>
        <td class="text-dim mono">${esc(e.user)}</td>
        <td class="text-dim mono">${esc(e.group)}</td>
        <td class="mono ${(e.mode & 0o111) ? 'fm-perm-exec' : 'text-dim'}">${esc(e.permissions)}</td>
        <td class="text-dim">${fmtDate(e.modified)}</td>
        <td>
          <div class="fm-acts">
          ${dl}
          <button class="fm-act" data-act="rename" data-path="${esc(e.path)}" title="${t('fm.rename')}"><i class="fa-solid fa-pen"></i></button>
          <button class="fm-act" data-act="stat" data-path="${esc(e.path)}" title="${t('fm.properties')}"><i class="fa-solid fa-circle-info"></i></button>
          <button class="fm-act fm-act-danger" data-act="delete" data-path="${esc(e.path)}" data-dir-flag="${e.is_dir ? '1' : '0'}" title="${t('common.delete')}"><i class="fa-solid fa-trash"></i></button>
          </div>
        </td>
      </tr>`;
  }).join('');
  return `
    <table class="fm-table">
      <thead><tr><th>${t('common.name')}</th><th>${t('common.size')}</th><th>${t('fm.owner')}</th><th>${t('fm.group')}</th><th>${t('fm.permissions')}</th><th>${t('fm.modified')}</th><th>${t('common.actions')}</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

function gridHtml(entries) {
  if (!entries.length) return '<div class="empty">' + t('fm.emptyDir') + '</div>';
  const cards = entries.map((e) => {
    const icon = fileIcon(e);
    const click = e.is_dir ? `data-dir="${esc(e.path)}"` : '';
    return `
      <div class="fm-grid-item ${e.is_dir ? 'fm-openable' : ''}" ${click}>
        <div class="fm-grid-ico">${icon}</div>
        <div class="fm-grid-name" title="${esc(e.name)}">${esc(e.name)}</div>
        <div class="fm-grid-meta mono">${e.is_dir ? t('fm.folder') : fmtBytes(e.size)}</div>
        <div class="fm-grid-acts">
          ${e.is_dir ? '' : `<button class="fm-act" data-act="download" data-path="${esc(e.path)}" title="${t('fm.download')}"><i class="fa-solid fa-download"></i></button>`}
          <button class="fm-act" data-act="rename" data-path="${esc(e.path)}" title="${t('fm.rename')}"><i class="fa-solid fa-pen"></i></button>
          <button class="fm-act" data-act="stat" data-path="${esc(e.path)}" title="${t('fm.properties')}"><i class="fa-solid fa-circle-info"></i></button>
          <button class="fm-act fm-act-danger" data-act="delete" data-path="${esc(e.path)}" data-dir-flag="${e.is_dir ? '1' : '0'}" title="${t('common.delete')}"><i class="fa-solid fa-trash"></i></button>
        </div>
      </div>`;
  }).join('');
  return `<div class="fm-grid">${cards}</div>`;
}

// ===== Actions =====

async function onUploadPicked(ev) {
  const input = ev.target;
  const files = [...input.files];
  input.value = '';
  if (!files.length) return;
  for (const file of files) {
    try {
      await uploadFile(currentDir, file);
      toast(t('fm.uploaded', { name: file.name }));
    } catch (err) {
      toast(t('fm.uploadFailed', { name: file.name, msg: err.message || '' }), 'error');
    }
  }
  await loadListing(currentDir);
  // Refresh tree in case dirs changed (uploads are files, but keep fresh).
}

function uploadFile(dir, file) {
  const url = `/api/files/upload?path=${encodeURIComponent(dir)}&filename=${encodeURIComponent(file.name)}`;
  return fetch(url, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token()}`, 'Content-Type': 'application/octet-stream' },
    body: file,
  }).then(async (res) => {
    const text = await res.text();
    const data = text ? JSON.parse(text) : null;
    if (!res.ok) throw { status: res.status, message: (data && data.message) || t('fm.uploadFailedStatus', { status: res.status }) };
    return data;
  });
}

async function downloadFile(path) {
  try {
    const res = await fetch(`/api/files/download?path=${encodeURIComponent(path)}`, {
      headers: { Authorization: `Bearer ${token()}` },
    });
    if (res.status === 401) {
      sessionStorage.removeItem('fwp_token');
      location.hash = '#/login';
      return;
    }
    if (!res.ok) {
      const data = await res.json().catch(() => null);
      throw { message: (data && data.message) || t('fm.downloadFailedStatus', { status: res.status }) };
    }
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = path.split('/').filter(Boolean).pop() || 'download';
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    toast(t('fm.downloadStarted'));
  } catch (err) {
    toast(t('fm.downloadFailed', { msg: err.message || '' }), 'error');
  }
}

async function onMkdir() {
  const name = await promptText(t('fm.mkdir'), t('fm.mkdirLabel'), '');
  if (!name) return;
  const target = joinPath(currentDir, name);
  try {
    await api.post(`/api/files/mkdir?path=${encodeURIComponent(target)}`);
    toast(t('fm.mkdirDone'));
    invalidateTree(currentDir);
    await refreshTree();
    await loadListing(currentDir);
  } catch (err) {
    toast(t('fm.mkdirFailed', { msg: err.message || '' }), 'error');
  }
}

async function onRename(path) {
  const oldName = path.split('/').filter(Boolean).pop() || '';
  const newName = await promptText(t('fm.rename'), t('fm.renameLabel'), oldName);
  if (!newName || newName === oldName) return;
  const parent = path.split('/').filter(Boolean).slice(0, -1).join('/') || '/';
  const target = joinPath(parent, newName);
  try {
    await api.post(`/api/files/rename?from=${encodeURIComponent(path)}&to=${encodeURIComponent(target)}`);
    toast(t('fm.renameDone'));
    invalidateTree(parent);
    await refreshTree();
    await loadListing(currentDir);
  } catch (err) {
    toast(t('fm.renameFailed', { msg: err.message || '' }), 'error');
  }
}

async function onDelete(path, isDir) {
  const name = path.split('/').filter(Boolean).pop() || path;
  const ok = await confirmDialog(
    t('common.delete'),
    isDir ? t('fm.deleteConfirmDir', { name }) : t('fm.deleteConfirmFile', { name }),
  );
  if (!ok) return;
  try {
    await api.del(`/api/files?path=${encodeURIComponent(path)}`);
    toast(t('fm.deleteDone'));
    invalidateTree(path);
    await refreshTree();
    await loadListing(currentDir);
  } catch (err) {
    toast(t('fm.deleteFailed', { msg: err.message || '' }), 'error');
  }
}

async function onStat(path) {
  let info;
  try {
    info = await api.get(`/api/files/stat?path=${encodeURIComponent(path)}`);
  } catch (err) {
    toast(t('fm.statReadFailed', { msg: err.message || '' }), 'error');
    return;
  }
  showStatModal(info);
}

function showStatModal(info) {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  const kind = info.is_dir ? t('fm.folder') : info.is_symlink ? t('fm.symlink') : t('fm.file');
  overlay.innerHTML = `
    <div class="modal" style="max-width:560px;">
      <h3>${t('fm.statTitle', { name: esc(info.name) })}</h3>
      <div class="fm-stat-grid">
        ${statRow(t('fm.path'), info.path, 'mono')}
        ${statRow(t('common.type'), kind)}
        ${info.symlink_target ? statRow(t('fm.target'), info.symlink_target, 'mono') : ''}
        ${statRow(t('common.size'), info.is_dir ? '—' : t('fm.sizeVal', { fmt: fmtBytes(info.size), bytes: info.size.toLocaleString() }), 'mono')}
        <div class="fm-stat-row">
          <div class="fm-stat-label">${t('fm.permissions')}</div>
          <div class="fm-stat-val"><span class="mono">${esc(info.permissions)}</span><button class="fm-act" data-act="chmod" style="margin-left:6px;" title="${t('fm.editPermissions')}"><i class="fa-solid fa-pen"></i></button></div>
        </div>
        <div class="fm-stat-row">
          <div class="fm-stat-label">${t('fm.owner')}</div>
          <div class="fm-stat-val mono">${esc(info.user)} (${info.uid}) / ${esc(info.group)} (${info.gid})<button class="fm-act" data-act="chown" style="margin-left:6px;" title="${t('fm.editOwner')}"><i class="fa-solid fa-pen"></i></button></div>
        </div>
        ${statRow(t('fm.inode'), `${info.inode}`, 'mono')}
        ${statRow(t('fm.hardLinks'), `${info.nlink}`, 'mono')}
        ${statRow(t('fm.modified'), fmtDate(info.modified))}
        ${statRow(t('fm.accessed'), fmtDate(info.accessed))}
        ${statRow(t('fm.changed'), fmtDate(info.changed))}
        ${statRow(t('fm.blocks'), info.blocks ? `${fmtBytes(info.blocks * 512)}` : '—', 'mono')}
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="close">${t('common.close')}</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', async (e) => {
    if (e.target === overlay || e.target.dataset.act === 'close') {
      overlay.remove();
    } else if (e.target.dataset.act === 'chmod') {
      overlay.remove();
      await editPermissions(info);
    } else if (e.target.dataset.act === 'chown') {
      overlay.remove();
      await editOwner(info);
    }
  });
}

async function editPermissions(info) {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  const mode = info.mode & 0o7777;
  const rows = [
    { label: t('fm.owner'), bits: { r: 0o400, w: 0o200, x: 0o100 }, special: { s: 0o4000, label: 'setuid' } },
    { label: t('fm.group'), bits: { r: 0o040, w: 0o020, x: 0o010 }, special: { s: 0o2000, label: 'setgid' } },
    { label: t('fm.other'), bits: { r: 0o004, w: 0o002, x: 0o001 }, special: { s: 0o1000, label: 'sticky' } },
  ];
  const gridHtml = rows.map((row) => `
    <div class="fm-perm-row">
      <span class="fm-perm-who">${row.label}</span>
      <label class="fm-perm-check"><input type="checkbox" data-bit="${row.bits.r}" ${mode & row.bits.r ? 'checked' : ''}/>${t('fm.read')}</label>
      <label class="fm-perm-check"><input type="checkbox" data-bit="${row.bits.w}" ${mode & row.bits.w ? 'checked' : ''}/>${t('fm.write')}</label>
      <label class="fm-perm-check"><input type="checkbox" data-bit="${row.bits.x}" ${mode & row.bits.x ? 'checked' : ''}/>${t('fm.execute')}</label>
      <label class="fm-perm-check"><input type="checkbox" data-bit="${row.special.s}" ${mode & row.special.s ? 'checked' : ''}/>${row.special.label}</label>
    </div>`).join('');

  overlay.innerHTML = `
    <div class="modal">
      <h3>${t('fm.editPermissions')} — ${esc(info.name)}</h3>
      <div class="fm-perm-grid">${gridHtml}</div>
      <div class="fm-perm-preview">
        <span class="text-dim">${t('fm.permissions')}:</span>
        <span class="mono" id="fm-perm-str">${esc(permStringFull(mode))}</span>
        <span class="text-dim">${t('fm.octalMode')}:</span>
        <span class="mono" id="fm-perm-oct">${octStr(mode)}</span>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
        <button data-act="ok">${t('common.ok')}</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);

  const previewStr = overlay.querySelector('#fm-perm-str');
  const previewOct = overlay.querySelector('#fm-perm-oct');

  function readMode() {
    let m = 0;
    overlay.querySelectorAll('[data-bit]').forEach((cb) => {
      if (cb.checked) m |= parseInt(cb.getAttribute('data-bit'), 10);
    });
    return m;
  }
  overlay.querySelectorAll('[data-bit]').forEach((cb) => {
    cb.addEventListener('change', () => {
      const m = readMode();
      previewStr.textContent = permStringFull(m);
      previewOct.textContent = octStr(m);
    });
  });

  return new Promise((resolve) => {
    overlay.addEventListener('click', async (e) => {
      if (e.target === overlay || e.target.dataset.act === 'cancel') {
        overlay.remove();
        resolve();
      } else if (e.target.dataset.act === 'ok') {
        const newMode = readMode();
        overlay.remove();
        try {
          await api.put(`/api/files/chmod?path=${encodeURIComponent(info.path)}`, { mode: newMode });
          toast(t('fm.permSaved'));
          await loadListing(currentDir);
        } catch (err) {
          toast(t('fm.saveFailed', { msg: err.message || '' }), 'error');
        }
        resolve();
      }
    });
  });
}

// Build a 9-char ls-style permission string (without leading type char) from a numeric mode.
function permStringFull(mode) {
  const chars = [
    [0o4000, 0o100, 's', 'S', 'r'],
    [0o2000, 0o010, 's', 'S', 'r'],
  ];
  let s = '';
  // owner
  s += mode & 0o400 ? 'r' : '-';
  s += mode & 0o200 ? 'w' : '-';
  s += mode & 0o4000 ? (mode & 0o100 ? 's' : 'S') : (mode & 0o100 ? 'x' : '-');
  // group
  s += mode & 0o040 ? 'r' : '-';
  s += mode & 0o020 ? 'w' : '-';
  s += mode & 0o2000 ? (mode & 0o010 ? 's' : 'S') : (mode & 0o010 ? 'x' : '-');
  // other
  s += mode & 0o004 ? 'r' : '-';
  s += mode & 0o002 ? 'w' : '-';
  s += mode & 0o1000 ? (mode & 0o001 ? 't' : 'T') : (mode & 0o001 ? 'x' : '-');
  return s;
}

function octStr(mode) {
  return mode.toString(8).padStart(4, '0');
}

async function editOwner(info) {
  let accts;
  try {
    accts = await api.get('/api/files/accounts');
  } catch (err) {
    toast(t('fm.saveFailed', { msg: err.message || '' }), 'error');
    return;
  }
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  const userOpts = accts.users
    .map((u) => `<option value="${u.id}" ${u.id === info.uid ? 'selected' : ''}>${esc(u.name)} (${u.id})</option>`)
    .join('');
  const groupOpts = accts.groups
    .map((g) => `<option value="${g.id}" ${g.id === info.gid ? 'selected' : ''}>${esc(g.name)} (${g.id})</option>`)
    .join('');
  overlay.innerHTML = `
    <div class="modal">
      <h3>${t('fm.editOwner')} — ${esc(info.name)}</h3>
      <div class="field">
        <label>${t('fm.user')}</label>
        <select id="fm-chown-user"><option value="">— ${t('common.unknown')} —</option>${userOpts}</select>
      </div>
      <div class="field">
        <label>${t('fm.group')}</label>
        <select id="fm-chown-group"><option value="">— ${t('common.unknown')} —</option>${groupOpts}</select>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
        <button data-act="ok">${t('common.ok')}</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  const finish = () => overlay.remove();
  return new Promise((resolve) => {
    overlay.addEventListener('click', async (e) => {
      if (e.target === overlay || e.target.dataset.act === 'cancel') {
        finish();
        resolve();
      } else if (e.target.dataset.act === 'ok') {
        const uidVal = overlay.querySelector('#fm-chown-user').value;
        const gidVal = overlay.querySelector('#fm-chown-group').value;
        const body = {};
        if (uidVal !== '') body.uid = parseInt(uidVal, 10);
        if (gidVal !== '') body.gid = parseInt(gidVal, 10);
        try {
          await api.put(`/api/files/chown?path=${encodeURIComponent(info.path)}`, body);
          toast(t('fm.ownerSaved'));
          finish();
          await loadListing(currentDir);
        } catch (err) {
          toast(t('fm.saveFailed', { msg: err.message || '' }), 'error');
        }
        resolve();
      }
    });
  });
}

function statRow(label, value, cls = '') {
  return `
    <div class="fm-stat-row">
      <div class="fm-stat-label">${label}</div>
      <div class="fm-stat-val ${cls}">${esc(value)}</div>
    </div>`;
}

// ===== Text-input dialog =====

function promptText(title, label, defaultVal) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.innerHTML = `
      <div class="modal">
        <h3>${esc(title)}</h3>
        <div class="field">
          <label>${esc(label)}</label>
          <input id="fm-prompt-input" type="text" value="${esc(defaultVal)}" />
        </div>
        <div class="modal-actions">
          <button class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
          <button data-act="ok">${t('common.ok')}</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);
    const input = overlay.querySelector('#fm-prompt-input');
    input.focus();
    input.select();
    const finish = (val) => {
      overlay.remove();
      resolve(val);
    };
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay || e.target.dataset.act === 'cancel') finish(null);
      else if (e.target.dataset.act === 'ok') finish(input.value.trim());
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') finish(input.value.trim());
      else if (e.key === 'Escape') finish(null);
    });
  });
}

// ===== Tree refresh helpers =====

/// Drop cached children for `path` and its descendants so the tree reloads.
function invalidateTree(path) {
  const toRemove = [];
  for (const key of treeChildren.keys()) {
    if (key === path || key.startsWith(path + '/')) toRemove.push(key);
  }
  toRemove.forEach((k) => treeChildren.delete(k));
}

async function refreshTree() {
  // Reload ancestors' children of currentDir to reflect additions/removals.
  const parts = currentDir.split('/').filter(Boolean);
  const paths = [ROOT];
  let cur = '';
  for (const p of parts) {
    cur = cur + '/' + p;
    paths.push(cur);
  }
  for (const p of paths) {
    try {
      treeChildren.set(p, await fetchDirs(p));
    } catch {
      /* ignore */
    }
  }
  renderTree();
}

// ===== Utils =====

function joinPath(dir, name) {
  if (dir === ROOT) return ROOT + name;
  return dir + '/' + name;
}

function pathDepth(path) {
  if (path === ROOT) return 0;
  return path.split('/').filter(Boolean).length;
}

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0;
  let v = b;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}

function fmtDate(ts) {
  if (!ts) return '—';
  const d = new Date(ts * 1000);
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s == null ? '' : String(s);
  return d.innerHTML;
}
