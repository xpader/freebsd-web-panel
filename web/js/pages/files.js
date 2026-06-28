// File manager — directory tree (left) + file listing (right) with
// list/grid views, upload, download, rename, delete, properties.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';

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
  if (e.is_dir) return '📁';
  if (e.is_symlink) return '↪️';
  const ext = (e.name.split('.').pop() || '').toLowerCase();
  const map = {
    txt: '📄', log: '📜', md: '📝', conf: '⚙️', json: '🔧',
    png: '🖼️', jpg: '🖼️', jpeg: '🖼️', gif: '🖼️', webp: '🖼️', svg: '🖼️', bmp: '🖼️',
    mp4: '🎬', mkv: '🎬', avi: '🎬', mov: '🎬', webm: '🎬',
    mp3: '🎵', wav: '🎵', flac: '🎵', ogg: '🎵',
    zip: '🗜️', gz: '🗜️', tar: '🗜️', xz: '🗜️', bz2: '🗜️', '7z': '🗜️', zst: '🗜️', iso: '💿',
    pdf: '📕',
    sh: '💻', py: '💻', pl: '💻', rb: '💻', js: '💻', rs: '💻', c: '💻', h: '💻',
  };
  return map[ext] || '📄';
}

// Folder icon for the tree: root vs nested.
function treeIcon(path) {
  return path === ROOT ? '🗂️' : '📁';
}

export async function renderFiles(app) {
  renderLayout(app, '/filesystem/files', `
    <div class="page-header">
      <h1>文件管理器</h1>
      <p>浏览、上传、下载、重命名、删除文件与目录</p>
    </div>
    <div class="fm-wrap">
      <div class="fm-tree">
        <div class="fm-tree-head">目录</div>
        <div id="fm-tree-body"><span class="spinner"></span></div>
      </div>
      <div class="fm-main" id="fm-main"><div class="empty"><span class="spinner"></span> 加载中…</div></div>
    </div>
    <input type="file" id="fm-upload-input" multiple style="display:none" />
  `);

  document.getElementById('fm-upload-input').addEventListener('change', onUploadPicked);

  try {
    await initTree();
    await openDir(currentDir);
  } catch (err) {
    document.getElementById('fm-main').innerHTML =
      `<div class="empty">加载失败：${esc(err.message || '')}</div>`;
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
  const arrow = !hasLoaded ? '▸' : loaded.length > 0 ? (isExpanded ? '▾' : '▸') : '';
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
  el.innerHTML = '<div class="empty"><span class="spinner"></span> 加载中…</div>';
  try {
    lastEntries = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
    renderListing(path, lastEntries);
  } catch (err) {
    el.innerHTML = `<div class="empty">加载失败：${esc(err.message || '')}</div>`;
  }
}

function renderListing(path, entries) {
  const el = document.getElementById('fm-main');
  el.innerHTML = `
    <div class="fm-toolbar">
      <div class="fm-breadcrumb" id="fm-breadcrumb">${breadcrumbHtml(path)}</div>
      <div class="fm-actions">
        <button class="btn-secondary btn-sm" id="fm-upload-btn">⤒ 上传</button>
        <button class="btn-secondary btn-sm" id="fm-mkdir-btn">+ 新建文件夹</button>
        <div class="fm-view-toggle">
          <button class="btn-secondary btn-sm ${viewMode === 'list' ? 'active-range' : ''}" data-view="list">☰ 列表</button>
          <button class="btn-secondary btn-sm ${viewMode === 'grid' ? 'active-range' : ''}" data-view="grid">▦ 网格</button>
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
  if (!entries.length) return '<div class="empty">空目录</div>';
  const rows = entries.map((e) => {
    const icon = fileIcon(e);
    const dl = e.is_dir ? '' : `<button class="fm-act" data-act="download" data-path="${esc(e.path)}" title="下载">⤓</button>`;
    return `
      <tr>
        <td class="fm-name-cell">
          <span class="fm-row-ico">${icon}</span>
          ${e.is_dir
            ? `<a class="fm-name-link" data-dir="${esc(e.path)}">${esc(e.name)}</a>`
            : `<span class="fm-name">${esc(e.name)}</span>`}
        </td>
        <td class="mono">${e.is_dir ? '—' : fmtBytes(e.size)}</td>
        <td class="text-dim mono">${esc(e.permissions)}</td>
        <td class="text-dim">${fmtDate(e.modified)}</td>
        <td class="fm-acts">
          ${dl}
          <button class="fm-act" data-act="rename" data-path="${esc(e.path)}" title="重命名">✎</button>
          <button class="fm-act" data-act="stat" data-path="${esc(e.path)}" title="属性">ℹ</button>
          <button class="fm-act fm-act-danger" data-act="delete" data-path="${esc(e.path)}" data-dir-flag="${e.is_dir ? '1' : '0'}" title="删除">🗑</button>
        </td>
      </tr>`;
  }).join('');
  return `
    <table class="fm-table">
      <thead><tr><th>名称</th><th>大小</th><th>权限</th><th>修改时间</th><th>操作</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

function gridHtml(entries) {
  if (!entries.length) return '<div class="empty">空目录</div>';
  const cards = entries.map((e) => {
    const icon = fileIcon(e);
    const click = e.is_dir ? `data-dir="${esc(e.path)}"` : '';
    return `
      <div class="fm-grid-item ${e.is_dir ? 'fm-openable' : ''}" ${click}>
        <div class="fm-grid-ico">${icon}</div>
        <div class="fm-grid-name" title="${esc(e.name)}">${esc(e.name)}</div>
        <div class="fm-grid-meta mono">${e.is_dir ? '文件夹' : fmtBytes(e.size)}</div>
        <div class="fm-grid-acts">
          ${e.is_dir ? '' : `<button class="fm-act" data-act="download" data-path="${esc(e.path)}" title="下载">⤓</button>`}
          <button class="fm-act" data-act="rename" data-path="${esc(e.path)}" title="重命名">✎</button>
          <button class="fm-act" data-act="stat" data-path="${esc(e.path)}" title="属性">ℹ</button>
          <button class="fm-act fm-act-danger" data-act="delete" data-path="${esc(e.path)}" data-dir-flag="${e.is_dir ? '1' : '0'}" title="删除">🗑</button>
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
      toast(`已上传 ${file.name}`);
    } catch (err) {
      toast(`上传失败 ${file.name}：${err.message || ''}`, 'error');
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
    if (!res.ok) throw { status: res.status, message: (data && data.message) || `上传失败 (${res.status})` };
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
      throw { message: (data && data.message) || `下载失败 (${res.status})` };
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
    toast('下载已开始');
  } catch (err) {
    toast(`下载失败：${err.message || ''}`, 'error');
  }
}

async function onMkdir() {
  const name = await promptText('新建文件夹', '文件夹名称', '');
  if (!name) return;
  const target = joinPath(currentDir, name);
  try {
    await api.post(`/api/files/mkdir?path=${encodeURIComponent(target)}`);
    toast('文件夹已创建');
    invalidateTree(currentDir);
    await refreshTree();
    await loadListing(currentDir);
  } catch (err) {
    toast(`创建失败：${err.message || ''}`, 'error');
  }
}

async function onRename(path) {
  const oldName = path.split('/').filter(Boolean).pop() || '';
  const newName = await promptText('重命名', '新名称', oldName);
  if (!newName || newName === oldName) return;
  const parent = path.split('/').filter(Boolean).slice(0, -1).join('/') || '/';
  const target = joinPath(parent, newName);
  try {
    await api.post(`/api/files/rename?from=${encodeURIComponent(path)}&to=${encodeURIComponent(target)}`);
    toast('已重命名');
    invalidateTree(parent);
    await refreshTree();
    await loadListing(currentDir);
  } catch (err) {
    toast(`重命名失败：${err.message || ''}`, 'error');
  }
}

async function onDelete(path, isDir) {
  const name = path.split('/').filter(Boolean).pop() || path;
  const ok = await confirmDialog(
    '删除',
    `确定删除 ${isDir ? '文件夹' : '文件'}「${name}」？${isDir ? '此操作将递归删除其所有内容。' : ''}`,
  );
  if (!ok) return;
  try {
    await api.del(`/api/files?path=${encodeURIComponent(path)}`);
    toast('已删除');
    invalidateTree(path);
    await refreshTree();
    await loadListing(currentDir);
  } catch (err) {
    toast(`删除失败：${err.message || ''}`, 'error');
  }
}

async function onStat(path) {
  let info;
  try {
    info = await api.get(`/api/files/stat?path=${encodeURIComponent(path)}`);
  } catch (err) {
    toast(`读取属性失败：${err.message || ''}`, 'error');
    return;
  }
  showStatModal(info);
}

function showStatModal(info) {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  const kind = info.is_dir ? '文件夹' : info.is_symlink ? '符号链接' : '文件';
  overlay.innerHTML = `
    <div class="modal" style="max-width:560px;">
      <h3>属性 — ${esc(info.name)}</h3>
      <div class="fm-stat-grid">
        ${statRow('路径', info.path, 'mono')}
        ${statRow('类型', kind)}
        ${info.symlink_target ? statRow('指向', info.symlink_target, 'mono') : ''}
        ${statRow('大小', info.is_dir ? '—' : `${fmtBytes(info.size)} (${info.size.toLocaleString()} B)`, 'mono')}
        ${statRow('权限', info.permissions, 'mono')}
        ${statRow('所有者', `UID ${info.uid} / GID ${info.gid}`, 'mono')}
        ${statRow('inode', `${info.inode}`, 'mono')}
        ${statRow('硬链接', `${info.nlink}`, 'mono')}
        ${statRow('修改时间', fmtDate(info.modified))}
        ${statRow('访问时间', fmtDate(info.accessed))}
        ${statRow('变更时间', fmtDate(info.changed))}
        ${statRow('占用块', info.blocks ? `${fmtBytes(info.blocks * 512)}` : '—', 'mono')}
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="close">关闭</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay || e.target.dataset.act === 'close') overlay.remove();
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
          <button class="btn-secondary" data-act="cancel">取消</button>
          <button data-act="ok">确定</button>
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
