<script>
export default { name: 'FileTreeRow' };
</script>

<script setup>
import { fileIcon } from '../../lib/fileTree.js';

const props = defineProps({
  entry: { type: Object, required: true },     // { path, name, is_dir, ... }
  depth: { type: Number, default: 0 },
  expanded: { type: Object, required: true },   // reactive Set
  treeChildren: { type: Object, required: true }, // reactive Map
  toggleExpand: { type: Function, required: true },
  selectedPath: { type: String, default: '' },   // currently active path
  isSelectable: { type: Boolean, default: false },
  clickableArrow: { type: Boolean, default: true }, // arrow toggles expand on click
});

const emit = defineEmits(['rowclick', 'rowdblclick']);
</script>

<template>
  <div class="ft-node">
    <div
      :class="['ft-row', { 'ft-active': selectedPath === entry.path, 'ft-selectable': isSelectable }]"
      :style="{ paddingLeft: depth * 14 + 6 + 'px' }"
      @click="emit('rowclick', entry)"
      @dblclick="emit('rowdblclick', entry)"
    >
      <span v-if="entry.is_dir" class="ft-arrow" @click.stop="clickableArrow ? toggleExpand(entry.path) : null">
        <i :class="expanded.has(entry.path) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
      </span>
      <span v-else class="ft-arrow ft-arrow-none"></span>
      <span class="ft-name">
        <span class="ft-ico"><i :class="fileIcon(entry)"></i></span>{{ entry.name }}
      </span>
    </div>
    <div v-if="entry.is_dir && expanded.has(entry.path) && (treeChildren.get(entry.path) || []).length" class="ft-children">
      <FileTreeRow
        v-for="child in (treeChildren.get(entry.path) || [])"
        :key="child.path"
        :entry="child"
        :depth="depth + 1"
        :expanded="expanded"
        :tree-children="treeChildren"
        :toggle-expand="toggleExpand"
        :selected-path="selectedPath"
        :is-selectable="isSelectable"
        :clickable-arrow="clickableArrow"
        @rowclick="(e) => emit('rowclick', e)"
        @rowdblclick="(e) => emit('rowdblclick', e)"
      />
    </div>
  </div>
</template>

<style>
/* Shared tree row styles — used by FilesPage (.fm-tree) and FilePicker (.fp-tree) */
.ft-node {}
.ft-row {
  display: flex; align-items: center; gap: 2px; cursor: default;
}
.ft-row:hover { background: var(--bg-elev2); }
.ft-selectable { cursor: pointer; }
.ft-arrow {
  width: 14px; text-align: center; color: var(--text-dim);
  font-size: 10px; user-select: none; flex-shrink: 0; cursor: pointer;
}
.ft-arrow:hover { color: var(--text); }
.ft-arrow-none { visibility: hidden; }
.ft-name {
  display: flex; align-items: center; gap: 6px;
  font-size: 13px; color: var(--text-dim);
  padding: 3px 6px; min-width: 0; flex: 1;
  overflow: hidden; text-overflow: ellipsis;
}
.ft-row:hover .ft-name { color: var(--text); }
.ft-row.ft-active {
  background: rgba(59, 130, 246, 0.15);
}
.ft-active .ft-name,
.ft-row.ft-active .ft-name { color: var(--accent); font-weight: 600; }
.ft-ico { font-size: 13px; opacity: 0.8; flex-shrink: 0; }
.ft-ico i { font-size: 13px; }
.ft-arrow i { font-size: 11px; }
.ft-children {}
</style>
