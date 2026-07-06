<script setup>
const model = defineModel({ type: String, default: '' });
const props = defineProps({
  tabs: { type: Array, required: true },
});
</script>

<template>
  <div class="tab-card card">
    <div class="tab-bar">
      <button
        v-for="tab in tabs"
        :key="tab.key"
        type="button"
        :class="['tab-btn', { active: model === tab.key }]"
        @click="model = tab.key"
      >{{ tab.label }}</button>
    </div>
    <div class="tab-content">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.tab-card {
  padding: 0;
  overflow: hidden;
}
.tab-bar {
  display: flex;
  gap: 0;
  padding: 0 12px;
  background: var(--bg-elev2);
  border-bottom: 1px solid var(--border);
}
.tab-btn {
  padding: 10px 18px;
  font-size: 13px;
  background: var(--bg-elev2);
  border: 1px solid transparent;
  border-bottom: none;
  border-radius: 6px 6px 0 0;
  color: var(--text-dim);
  cursor: pointer;
  white-space: nowrap;
  margin-top: 6px;
}
.tab-btn:hover {
  color: var(--text);
}
.tab-btn.active {
  background: var(--bg-elev);
  border-color: var(--border);
  border-bottom: 1px solid var(--bg-elev);
  color: var(--text);
  position: relative;
  margin-bottom: -1px;
}
.tab-content {
  padding: 20px;
}
</style>
