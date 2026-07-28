<script setup>
import { ref, onMounted, onUnmounted } from 'vue';

const model = defineModel({ type: String, default: '' });

const props = defineProps({
  options: { type: Array, default: () => [] },
  placeholder: { type: String, default: '' },
});

const open = ref(false);
const root = ref(null);

function toggle() {
  open.value = !open.value;
}

function select(value) {
  model.value = value;
  open.value = false;
}

function onClickOutside(e) {
  if (root.value && !root.value.contains(e.target)) {
    open.value = false;
  }
}

function isCurrent(opt) {
  return (opt.value ?? opt) === model.value;
}

onMounted(() => document.addEventListener('click', onClickOutside));
onUnmounted(() => document.removeEventListener('click', onClickOutside));
</script>

<template>
  <div class="combobox" ref="root">
    <input v-model="model" :placeholder="placeholder" />
    <button v-if="options.length" type="button" class="combobox-toggle" @click="toggle">
      <i :class="['fa-solid', open ? 'fa-caret-up' : 'fa-caret-down']"></i>
    </button>
    <ul v-if="open && options.length" class="combobox-dropdown">
      <li
        v-for="opt in options"
        :key="opt.value ?? opt"
        :class="{ current: isCurrent(opt) }"
        @click="select(opt.value ?? opt)"
      >{{ opt.label ?? opt }}</li>
    </ul>
  </div>
</template>

<style scoped>
.combobox { position: relative; flex: 1; }
.combobox input { width: 100%; }
.combobox-toggle {
  position: absolute; right: 4px; top: 50%; transform: translateY(-50%);
  background: none; border: none; cursor: pointer; padding: 2px 6px;
  color: var(--text-dim); font-size: 14px; line-height: 1;
}
.combobox-dropdown {
  position: absolute; top: 100%; left: 0; right: 0; z-index: 100;
  margin: 2px 0 0; padding: 4px 0; list-style: none;
  background: var(--bg-elev2); border: 1px solid var(--border);
  border-radius: 4px; max-height: 200px; overflow-y: auto;
  box-shadow: 0 4px 12px var(--shadow);
}
.combobox-dropdown li {
  padding: 6px 12px; cursor: pointer; font-size: 13px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.combobox-dropdown li:hover { background: var(--hover-bg); }
.combobox-dropdown li.current { color: var(--accent); font-weight: 600; }
.combobox-dropdown li.current::before {
  content: '\f00c'; font-family: 'Font Awesome 6 Free'; font-weight: 900;
  margin-right: 6px; font-size: 11px;
}
</style>
