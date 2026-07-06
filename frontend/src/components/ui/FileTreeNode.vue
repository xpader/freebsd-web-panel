<script>
export default { name: 'FileTreeNode' };
</script>

<script setup>
import { inject } from 'vue';

const props = defineProps({
  entry: { type: Object, required: true },
  depth: { type: Number, default: 0 },
});

const expanded = inject('fpExpanded');
const selected = inject('fpSelected');
const toggleExpand = inject('fpToggleExpand');
const filteredChildren = inject('fpFilteredChildren');
const isSelectable = inject('fpIsSelectable');
const selectEntry = inject('fpSelectEntry');
const fileIcon = inject('fpFileIcon');

function onRowClick(entry) {
  if (entry.is_dir) {
    toggleExpand(entry.path);
    if (isSelectable(entry)) selectEntry(entry);
  } else {
    selectEntry(entry);
  }
}
</script>

<template>
  <div class="fp-tree-node">
    <div
      :class="['fp-tree-row', { 'fp-active': selected === entry.path, 'fp-selectable': isSelectable(entry) }]"
      :style="{ paddingLeft: depth * 14 + 6 + 'px' }"
      @click="onRowClick(entry)"
      @dblclick="!entry.is_dir ? selectEntry(entry) : null"
    >
      <span v-if="entry.is_dir" class="fp-tree-arrow" @click.stop="toggleExpand(entry.path)">
        <i :class="expanded.has(entry.path) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
      </span>
      <span v-else class="fp-tree-arrow fp-tree-arrow-none"></span>
      <span class="fp-tree-name">
        <span class="fp-tree-ico"><i :class="fileIcon(entry)"></i></span>{{ entry.name }}
      </span>
    </div>
    <div v-if="entry.is_dir && expanded.has(entry.path) && filteredChildren(entry.path).length" class="fp-tree-children">
      <FileTreeNode
        v-for="child in filteredChildren(entry.path)"
        :key="child.path"
        :entry="child"
        :depth="depth + 1"
      />
    </div>
  </div>
</template>
