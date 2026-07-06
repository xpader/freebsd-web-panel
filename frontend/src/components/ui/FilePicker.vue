<script setup>
import { ref, reactive, computed, onMounted, provide, readonly } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';
import FileTreeNode from './FileTreeNode.vue';

const props = defineProps({
  mode: { type: String, default: 'dir' }, // 'dir' | 'file'
  accept: { type: Array, default: () => [] }, // e.g. ['.txz', '.iso']
  initialPath: { type: String, default: '/' },
});

const emit = defineEmits(['select', 'close']);
const { t } = useI18n();

const ROOT = '/';
const selected = ref(null);
const expanded = reactive(new Set([ROOT]));
const treeChildren = reactive(new Map());
const loading = ref(false);

const isDirMode = computed(() => props.mode === 'dir');

function fileIcon(e) {
  if (e.is_dir) return 'fa-solid fa-folder';
  if (e.is_symlink) return 'fa-solid fa-link';
  const ext = (e.name.split('.').pop() || '').toLowerCase();
  const map = {
    txt: 'fa-regular fa-file-lines', log: 'fa-regular fa-file-lines', md: 'fa-regular fa-file-lines',
    png: 'fa-regular fa-file-image', jpg: 'fa-regular fa-file-image', jpeg: 'fa-regular fa-file-image',
    gif: 'fa-regular fa-file-image', webp: 'fa-regular fa-file-image', svg: 'fa-regular fa-file-image',
    zip: 'fa-regular fa-file-zipper', gz: 'fa-regular fa-file-zipper', tar: 'fa-regular fa-file-zipper',
    xz: 'fa-regular fa-file-zipper', '7z': 'fa-regular fa-file-zipper',
    pdf: 'fa-regular fa-file-pdf',
    sh: 'fa-regular fa-file-code', py: 'fa-regular fa-file-code', js: 'fa-regular fa-file-code',
    rs: 'fa-regular fa-file-code', c: 'fa-regular fa-file-code', json: 'fa-regular fa-file-code',
  };
  return map[ext] || 'fa-regular fa-file';
}

function extMatch(name) {
  if (!props.accept.length) return true;
  return props.accept.some((ext) => name.toLowerCase().endsWith(ext.toLowerCase()));
}

function isSelectable(e) {
  if (isDirMode.value) return e.is_dir;
  return !e.is_dir;
}

function pathDepth(path) {
  if (path === ROOT) return 0;
  return path.split('/').filter(Boolean).length;
}

function basename(path) {
  return path.split('/').filter(Boolean).pop() || '/';
}

function filteredChildren(path) {
  const children = treeChildren.get(path);
  if (!children) return [];
  if (isDirMode.value) return children.filter((e) => e.is_dir);
  return children.filter((e) => e.is_dir || extMatch(e.name));
}

async function fetchChildren(path) {
  const list = await api.get(`/api/files/list?path=${encodeURIComponent(path)}`);
  return isDirMode.value ? list.filter((e) => e.is_dir) : list.filter((e) => e.is_dir || extMatch(e.name));
}

async function toggleExpand(path) {
  if (!treeChildren.has(path)) {
    try { treeChildren.set(path, await fetchChildren(path)); } catch { treeChildren.set(path, []); }
  }
  if (expanded.has(path)) expanded.delete(path);
  else expanded.add(path);
}

function selectEntry(entry) {
  if (!isSelectable(entry)) return;
  selected.value = entry.path;
}

function rowDblClickFile(entry) {
  if (!isDirMode.value && !entry.is_dir) {
    selected.value = entry.path;
    doSelect();
  }
}

async function ensureAncestors(path) {
  const parts = path.split('/').filter(Boolean);
  let cur = '';
  for (const part of parts) {
    cur = cur + '/' + part;
    if (!treeChildren.has(cur)) {
      try { treeChildren.set(cur, await fetchChildren(cur)); } catch { treeChildren.set(cur, []); }
    }
    expanded.add(cur);
  }
  expanded.add(ROOT);
}

function canConfirm() {
  if (!selected.value) return false;
  if (isDirMode.value) return true;
  const base = basename(selected.value);
  return extMatch(base);
}

function doSelect() {
  if (!canConfirm()) return;
  emit('select', selected.value);
}

function onOverlayClick(e) {
  if (e.target === e.currentTarget) emit('close');
}

// Provide functions for recursive FileTreeNode
provide('fpExpanded', expanded);
provide('fpSelected', readonly(selected));
provide('fpToggleExpand', toggleExpand);
provide('fpFilteredChildren', filteredChildren);
provide('fpIsSelectable', isSelectable);
provide('fpSelectEntry', selectEntry);
provide('fpFileIcon', fileIcon);

onMounted(async () => {
  loading.value = true;
  try {
    treeChildren.set(ROOT, await fetchChildren(ROOT));
    if (props.initialPath && props.initialPath !== ROOT) {
      await ensureAncestors(props.initialPath);
      selected.value = props.initialPath;
    }
  } catch { /* ignore */ } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="modal-overlay fp-overlay" @click="onOverlayClick">
    <div class="modal fp-modal">
      <div class="fp-header">
        <h3>{{ isDirMode ? t('fp.selectDir') : t('fp.selectFile') }}</h3>
        <div v-if="selected" class="fp-selected-path mono">{{ selected }}</div>
      </div>

      <div class="fp-tree">
        <div v-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
        <div v-else class="fp-tree-body">
          <!-- Root -->
          <div class="fp-tree-node">
            <div
              :class="['fp-tree-row', { 'fp-active': selected === ROOT, 'fp-selectable': isDirMode }]"
              style="padding-left:6px"
              @click="isDirMode ? (selected = ROOT) : toggleExpand(ROOT)"
            >
              <span class="fp-tree-arrow" @click.stop="toggleExpand(ROOT)">
                <i :class="expanded.has(ROOT) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
              </span>
              <span class="fp-tree-name">
                <span class="fp-tree-ico"><i class="fa-solid fa-folder-tree"></i></span>/
              </span>
            </div>
            <div v-if="expanded.has(ROOT)" class="fp-tree-children">
              <FileTreeNode
                v-for="child in filteredChildren(ROOT)"
                :key="child.path"
                :entry="child"
                :depth="1"
              />
            </div>
          </div>
        </div>
      </div>

      <div v-if="!isDirMode && accept.length" class="fp-filter-hint">
        <i class="fa-solid fa-filter"></i> {{ accept.join(', ') }}
      </div>

      <div class="modal-actions">
        <button class="btn-secondary" @click="emit('close')">{{ t('common.cancel') }}</button>
        <button @click="doSelect" :disabled="!canConfirm()">{{ t('common.confirm') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fp-overlay {
  z-index: 60;
}
.fp-modal {
  display: flex;
  flex-direction: column;
  max-height: 80vh;
  min-width: 500px;
  max-width: 700px;
}
.fp-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.fp-selected-path {
  font-size: 12px;
  color: var(--text-dim);
  background: var(--bg-elev);
  padding: 4px 8px;
  border-radius: 4px;
  max-width: 380px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.fp-tree {
  flex: 1;
  overflow: auto;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  min-height: 200px;
  max-height: 50vh;
}
.fp-tree-body {
  padding: 4px;
}
.fp-tree-children {
  /* no extra indentation; padding is computed per-row */
}
:deep(.fp-tree-row) {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 6px;
  border-radius: 4px;
  cursor: default;
  white-space: nowrap;
  font-size: 13px;
  transition: background 0.08s;
}
:deep(.fp-tree-row:hover) {
  background: var(--bg-elev);
}
:deep(.fp-tree-row.fp-selectable) {
  cursor: pointer;
}
:deep(.fp-tree-row.fp-active) {
  background: var(--accent);
  color: #fff;
}
:deep(.fp-tree-arrow) {
  width: 14px;
  text-align: center;
  font-size: 11px;
  cursor: pointer;
  color: var(--text-dim);
  flex-shrink: 0;
}
:deep(.fp-tree-arrow-none) {
  visibility: hidden;
}
:deep(.fp-tree-name) {
  display: flex;
  align-items: center;
  gap: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
}
:deep(.fp-tree-ico) {
  width: 14px;
  text-align: center;
  flex-shrink: 0;
}
.fp-filter-hint {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-dim);
}
</style>
