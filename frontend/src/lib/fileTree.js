// Shared utilities for file tree operations — used by FilesPage and FilePicker.

import { reactive } from 'vue';
import { api } from './api.js';

export const ROOT = '/';

const ICON_MAP = {
  txt: 'fa-regular fa-file-lines', log: 'fa-regular fa-file-lines', md: 'fa-regular fa-file-lines',
  png: 'fa-regular fa-file-image', jpg: 'fa-regular fa-file-image', jpeg: 'fa-regular fa-file-image',
  gif: 'fa-regular fa-file-image', webp: 'fa-regular fa-file-image', svg: 'fa-regular fa-file-image',
  mp4: 'fa-regular fa-file-video', mkv: 'fa-regular fa-file-video', avi: 'fa-regular fa-file-video',
  mp3: 'fa-regular fa-file-audio', wav: 'fa-regular fa-file-audio',
  zip: 'fa-regular fa-file-zipper', gz: 'fa-regular fa-file-zipper', tar: 'fa-regular fa-file-zipper',
  xz: 'fa-regular fa-file-zipper', '7z': 'fa-regular fa-file-zipper',
  pdf: 'fa-regular fa-file-pdf',
  sh: 'fa-regular fa-file-code', py: 'fa-regular fa-file-code', js: 'fa-regular fa-file-code',
  rs: 'fa-regular fa-file-code', c: 'fa-regular fa-file-code', json: 'fa-regular fa-file-code',
};

export function fileIcon(e) {
  if (e.is_dir) return 'fa-solid fa-folder';
  if (e.is_symlink) return 'fa-solid fa-link';
  const ext = (e.name.split('.').pop() || '').toLowerCase();
  return ICON_MAP[ext] || 'fa-regular fa-file';
}

export function pathDepth(path) {
  if (path === ROOT) return 0;
  return path.split('/').filter(Boolean).length;
}

export function basename(path) {
  return path.split('/').filter(Boolean).pop() || '/';
}

export function joinPath(dir, name) {
  if (dir === ROOT) return ROOT + name;
  return dir + '/' + name;
}

export function extMatch(name, accept) {
  if (!accept || !accept.length) return true;
  return accept.some((ext) => name.toLowerCase().endsWith(ext.toLowerCase()));
}

/**
 * Create a reactive tree state with lazy-loading children.
 * @param {object} opts
 * @param {function} opts.filterFn  - (entry) => boolean, filter which entries to keep
 * @returns { object } { expanded, treeChildren, toggleExpand, ensureAncestors, getChildren, invalidate, refreshAll }
 */
export function createTreeState({ filterFn } = {}) {
  const expanded = reactive(new Set());
  const treeChildren = reactive(new Map());

  async function fetchDir(path) {
    const list = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
    return filterFn ? list.filter(filterFn) : list;
  }

  async function toggleExpand(path) {
    if (!treeChildren.has(path)) {
      try { treeChildren.set(path, await fetchDir(path)); } catch { treeChildren.set(path, []); }
    }
    if (expanded.has(path)) expanded.delete(path);
    else expanded.add(path);
  }

  function getChildren(path) {
    return treeChildren.get(path);
  }

  async function ensureAncestors(path) {
    const parts = path.split('/').filter(Boolean);
    let cur = '';
    for (const part of parts) {
      cur = cur + '/' + part;
      if (!treeChildren.has(cur)) {
        try { treeChildren.set(cur, await fetchDir(cur)); } catch { treeChildren.set(cur, []); }
      }
      expanded.add(cur);
    }
    expanded.add(ROOT);
  }

  function invalidate(path) {
    for (const key of [...treeChildren.keys()]) {
      if (key === path || key.startsWith(path + '/')) treeChildren.delete(key);
    }
  }

  async function refreshAll(paths) {
    for (const p of paths) {
      try { treeChildren.set(p, await fetchDir(p)); } catch {}
    }
  }

  return { expanded, treeChildren, toggleExpand, ensureAncestors, getChildren, invalidate, refreshAll };
}
