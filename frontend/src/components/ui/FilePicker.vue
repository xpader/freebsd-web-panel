<script setup>
import { ref, reactive, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';

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
  return !e.is_dir && extMatch(e.name);
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

function rowClick(e) {
  if (!isSelectable(e)) return;
  selected.value = e.path;
}

function rowDblClick(e) {
  if (!isDirMode.value && !e.is_dir && extMatch(e.name)) {
    selected.value = e.path;
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
  return extMatch(basename(selected.value));
}

function doSelect() {
  if (!canConfirm()) return;
  emit('select', selected.value);
}

function onOverlayClick(e) {
  if (e.target === e.currentTarget) emit('close');
}

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
              @click="isDirMode ? (selected = ROOT) : null"
            >
              <span class="fp-tree-arrow" @click.stop="toggleExpand(ROOT)">
                <i :class="expanded.has(ROOT) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
              </span>
              <span class="fp-tree-name">
                <span class="fp-tree-ico"><i class="fa-solid fa-folder-tree"></i></span>/
              </span>
            </div>
            <div v-if="expanded.has(ROOT)" class="fp-tree-children">
              <template v-for="d in filteredChildren(ROOT)" :key="d.path">
                <div class="fp-tree-node">
                  <div
                    :class="['fp-tree-row', { 'fp-active': selected === d.path, 'fp-selectable': isSelectable(d) }]"
                    :style="{ paddingLeft: pathDepth(d.path) * 14 + 6 + 'px' }"
                    @click="rowClick(d)"
                    @dblclick="rowDblClick(d)"
                  >
                    <span class="fp-tree-arrow" @click.stop="toggleExpand(d.path)">
                      <i v-if="expanded.has(d.path)" class="fa-solid fa-caret-down"></i>
                      <i v-else class="fa-solid fa-caret-right"></i>
                    </span>
                    <span class="fp-tree-name">
                      <span class="fp-tree-ico"><i :class="fileIcon(d)"></i></span>{{ d.name }}
                    </span>
                  </div>
                  <div v-if="expanded.has(d.path) && treeChildren.has(d.path)" class="fp-tree-children">
                    <template v-for="c in filteredChildren(d.path)" :key="c.path">
                      <div class="fp-tree-node">
                        <div
                          :class="['fp-tree-row', { 'fp-active': selected === c.path, 'fp-selectable': isSelectable(c) }]"
                          :style="{ paddingLeft: pathDepth(c.path) * 14 + 6 + 'px' }"
                          @click="rowClick(c)"
                          @dblclick="rowDblClick(c)"
                        >
                          <span v-if="c.is_dir" class="fp-tree-arrow" @click.stop="toggleExpand(c.path)">
                            <i v-if="expanded.has(c.path)" class="fa-solid fa-caret-down"></i>
                            <i v-else class="fa-solid fa-caret-right"></i>
                          </span>
                          <span v-else class="fp-tree-arrow fp-tree-arrow-none"></span>
                          <span class="fp-tree-name">
                            <span class="fp-tree-ico"><i :class="fileIcon(c)"></i></span>{{ c.name }}
                          </span>
                        </div>
                        <div v-if="c.is_dir && expanded.has(c.path) && treeChildren.has(c.path)" class="fp-tree-children">
                          <template v-for="gc in filteredChildren(c.path)" :key="gc.path">
                            <div class="fp-tree-node">
                              <div
                                :class="['fp-tree-row', { 'fp-active': selected === gc.path, 'fp-selectable': isSelectable(gc) }]"
                                :style="{ paddingLeft: pathDepth(gc.path) * 14 + 6 + 'px' }"
                                @click="rowClick(gc)"
                                @dblclick="rowDblClick(gc)"
                              >
                                <span v-if="gc.is_dir" class="fp-tree-arrow" @click.stop="toggleExpand(gc.path)">
                                  <i v-if="expanded.has(gc.path)" class="fa-solid fa-caret-down"></i>
                                  <i v-else class="fa-solid fa-caret-right"></i>
                                </span>
                                <span v-else class="fp-tree-arrow fp-tree-arrow-none"></span>
                                <span class="fp-tree-name">
                                  <span class="fp-tree-ico"><i :class="fileIcon(gc)"></i></span>{{ gc.name }}
                                </span>
                              </div>
                              <div v-if="gc.is_dir && expanded.has(gc.path) && treeChildren.has(gc.path)" class="fp-tree-children">
                                <template v-for="ggc in filteredChildren(gc.path)" :key="ggc.path">
                                  <div class="fp-tree-node">
                                    <div
                                      :class="['fp-tree-row', { 'fp-active': selected === ggc.path, 'fp-selectable': isSelectable(ggc) }]"
                                      :style="{ paddingLeft: pathDepth(ggc.path) * 14 + 6 + 'px' }"
                                      @click="rowClick(ggc)"
                                      @dblclick="rowDblClick(ggc)"
                                    >
                                      <span v-if="ggc.is_dir" class="fp-tree-arrow" @click.stop="toggleExpand(ggc.path)">
                                        <i v-if="expanded.has(ggc.path)" class="fa-solid fa-caret-down"></i>
                                        <i v-else class="fa-solid fa-caret-right"></i>
                                      </span>
                                      <span v-else class="fp-tree-arrow fp-tree-arrow-none"></span>
                                      <span class="fp-tree-name">
                                        <span class="fp-tree-ico"><i :class="fileIcon(ggc)"></i></span>{{ ggc.name }}
                                      </span>
                                    </div>
                                  </div>
                                </template>
                              </div>
                            </div>
                          </template>
                        </div>
                      </div>
                    </template>
                  </div>
                </div>
              </template>
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
  max-width: 560px;
  display: flex;
  flex-direction: column;
  max-height: 80vh;
}
.fp-header { margin-bottom: 12px; }
.fp-selected-path {
  font-size: 11px;
  color: var(--accent);
  margin-top: 4px;
  word-break: break-all;
}
.fp-tree {
  flex: 1;
  overflow: auto;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  min-height: 240px;
  max-height: calc(80vh - 200px);
}
.fp-tree-body { padding: 6px 0; }
.fp-tree-node {}
.fp-tree-row {
  display: flex;
  align-items: center;
  gap: 2px;
  cursor: default;
}
.fp-tree-row:hover { background: var(--bg-elev2); }
.fp-tree-row.fp-selectable { cursor: pointer; }
.fp-tree-arrow {
  width: 14px;
  text-align: center;
  color: var(--text-dim);
  font-size: 10px;
  user-select: none;
  flex-shrink: 0;
  cursor: pointer;
}
.fp-tree-arrow:hover { color: var(--text); }
.fp-tree-arrow-none { cursor: default; }
.fp-tree-name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-dim);
  padding: 3px 6px;
  min-width: 0;
  flex: 1;
}
.fp-tree-row:hover .fp-tree-name { color: var(--text); }
.fp-tree-row.fp-active .fp-tree-name {
  color: var(--accent);
  font-weight: 600;
}
.fp-tree-ico { font-size: 13px; opacity: 0.8; }
.fp-tree-ico i { font-size: 13px; }
.fp-tree-arrow i { font-size: 11px; }

.fp-filter-hint {
  font-size: 11px;
  color: var(--text-dim);
  padding: 6px 0;
  display: flex;
  align-items: center;
  gap: 4px;
}
</style>
