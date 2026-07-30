<script setup>
import { ref, reactive, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';
import { ROOT, fileIcon, basename, extMatch, createTreeState } from '../../lib/fileTree.js';
import FileTreeRow from './FileTreeRow.vue';

const props = defineProps({
  mode: { type: String, default: 'dir' }, // 'dir' | 'file'
  accept: { type: Array, default: () => [] }, // e.g. ['.txz', '.iso']
  initialPath: { type: String, default: '/' },
});

const emit = defineEmits(['select', 'close']);
const { t } = useI18n();

const selected = ref(null);
const loading = ref(false);

const isDirMode = computed(() => props.mode === 'dir');

const filterFn = computed(() => {
  if (isDirMode.value) return (e) => e.is_dir;
  return (e) => e.is_dir || extMatch(e.name, props.accept);
});

const { expanded, treeChildren, toggleExpand, ensureAncestors, getChildren } =
  createTreeState({ filterFn: (e) => true }); // filter applied in filteredChildren instead

function filteredChildren(path) {
  const children = getChildren(path);
  if (!children) return [];
  return children.filter(filterFn.value);
}

function isSelectable(e) {
  if (isDirMode.value) return e.is_dir;
  return !e.is_dir;
}

function rowClick(e) {
  if (e.is_dir) {
    toggleExpand(e.path);
    if (isSelectable(e)) selected.value = e.path;
  } else {
    if (isSelectable(e)) selected.value = e.path;
  }
}

function rowDblClick(e) {
  if (!isDirMode.value && !e.is_dir) {
    selected.value = e.path;
    doSelect();
  } else if (isDirMode.value && e.is_dir) {
    selected.value = e.path;
    doSelect();
  }
}

function canConfirm() {
  if (!selected.value) return false;
  if (isDirMode.value) return true;
  return extMatch(basename(selected.value), props.accept);
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
    const list = await api.get(`/api/files/list?path=${encodeURIComponent(ROOT)}`);
    treeChildren.set(ROOT, list);
    expanded.add(ROOT);
    if (props.initialPath && props.initialPath !== ROOT) {
      await ensureAncestors(props.initialPath);
      // Normalize trailing slashes so selected matches the canonical node path
      // (entry paths are slash-normalized by the backend).
      selected.value = props.initialPath.replace(/\/+$/, '') || ROOT;
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
          <div class="ft-node">
              <div
                :class="['ft-row', { 'ft-active': selected === ROOT, 'ft-selectable': isDirMode }]"
                style="padding-left:6px"
                @click="isDirMode ? (selected = ROOT) : null"
              >
                <span class="ft-arrow" @click.stop="toggleExpand(ROOT)">
                  <i :class="expanded.has(ROOT) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
                </span>
                <span class="ft-name">
                  <span class="ft-ico"><i class="fa-solid fa-folder-tree"></i></span>/
                </span>
              </div>
            <div v-if="expanded.has(ROOT)" class="ft-children">
              <FileTreeRow
                v-for="child in filteredChildren(ROOT)"
                :key="child.path"
                :entry="child"
                :depth="1"
                :expanded="expanded"
                :tree-children="treeChildren"
                :toggle-expand="toggleExpand"
                :selected-path="selected || ''"
                :is-selectable="isSelectable(child)"
                @rowclick="rowClick"
                @rowdblclick="rowDblClick"
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
.fp-overlay { z-index: 60; }
.fp-modal { display: flex; flex-direction: column; max-height: 80vh; min-width: 500px; max-width: 700px; }
.fp-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.fp-selected-path {
  font-size: 12px; color: var(--text-dim); background: var(--bg-elev);
  padding: 4px 8px; border-radius: 4px; max-width: 380px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.fp-tree {
  flex: 1; overflow: auto; background: var(--bg);
  border: 1px solid var(--border); border-radius: var(--radius);
  min-height: 200px; max-height: 50vh;
}
.fp-tree-body { padding: 4px; }
.fp-filter-hint { margin-top: 8px; font-size: 12px; color: var(--text-dim); }
</style>
