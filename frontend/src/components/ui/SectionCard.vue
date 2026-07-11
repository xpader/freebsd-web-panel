<script setup>
const model = defineModel({ type: String, default: '' });
const props = defineProps({
  tabs: { type: Array, required: true },
  expand: { type: Boolean, default: false },
});
</script>

<template>
  <div class="section-card card">
    <!-- Tab mode: clickable header, only active section rendered -->
    <template v-if="!expand">
      <div class="section-bar">
        <button
          v-for="tab in tabs"
          :key="tab.key"
          type="button"
          :class="['section-btn', { active: model === tab.key }]"
          @click="model = tab.key"
        >{{ tab.label }}</button>
      </div>
      <div class="section-content">
        <slot :active="model" />
      </div>
    </template>

    <!-- Expand mode: all sections visible, separated by headers -->
    <template v-else>
      <template v-for="(tab, idx) in tabs" :key="tab.key">
        <div class="section-header-stacked">{{ tab.label }}</div>
        <div class="section-content">
          <slot :active="tab.key" />
        </div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.section-card {
  padding: 0;
}

/* ---- Tab mode ---- */
.section-bar {
  display: flex;
  gap: 0;
  padding: 0 12px;
  background: var(--bg-elev2);
  border-bottom: 1px solid var(--border);
}
.section-btn {
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
.section-btn:hover {
  color: var(--text);
}
.section-btn.active {
  background: var(--bg-elev);
  border-color: var(--border);
  border-bottom: 1px solid var(--bg-elev);
  color: var(--text);
  position: relative;
  margin-bottom: -1px;
}

/* ---- Expand mode ---- */
.section-header-stacked {
  padding: 10px 20px;
  font-size: 13px;
  color: var(--text);
  background: var(--bg-elev2);
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}
.section-header-stacked:first-child {
  border-top: none;
}

/* ---- Shared ---- */
.section-content {
  padding: 20px;
}
</style>
