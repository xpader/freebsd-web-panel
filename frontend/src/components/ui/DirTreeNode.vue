<script>
export default { name: 'DirTreeNode' };
</script>

<script setup>
const props = defineProps({
  path: { type: String, required: true },
  name: { type: String, required: true },
  expanded: { type: Object, required: true }, // reactive Set
  treeChildren: { type: Object, required: true }, // reactive Map
  currentDir: { type: String, required: true },
  depth: { type: Number, default: 0 },
  toggleExpand: { type: Function, required: true },
  openDir: { type: Function, required: true },
});
</script>

<template>
  <div class="fm-tree-node">
    <div
      :class="['fm-tree-row', { active: currentDir === path }]"
      :style="{ paddingLeft: depth * 14 + 6 + 'px' }"
      @click="openDir(path)"
    >
      <span class="fm-tree-arrow" @click.stop="toggleExpand(path)">
        <i v-if="expanded.has(path)" class="fa-solid fa-caret-down"></i>
        <i v-else class="fa-solid fa-caret-right"></i>
      </span>
      <span class="fm-tree-name">
        <span class="fm-tree-ico"><i class="fa-solid fa-folder"></i></span>{{ name }}
      </span>
    </div>
    <div v-if="expanded.has(path) && treeChildren.has(path)" class="fm-tree-children">
      <DirTreeNode
        v-for="d in (treeChildren.get(path) || [])"
        :key="d.path"
        :path="d.path"
        :name="d.name"
        :expanded="expanded"
        :tree-children="treeChildren"
        :current-dir="currentDir"
        :depth="depth + 1"
        :toggle-expand="toggleExpand"
        :open-dir="openDir"
      />
    </div>
  </div>
</template>
