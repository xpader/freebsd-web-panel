<!--
StatusBar usage:

<StatusBar :items="[
  { title: 'Driver', value: 'ipfw' },
  { title: 'Status', value: 'Running', type: 'badge', status: 'ok' },
]">
  <template #actions><button>Refresh</button></template>
</StatusBar>

items: Array<{ title: string, value: string | number, type?: 'text' | 'badge', status?: 'ok' | 'error' | 'warning' | 'inactive' }>
- text: normal value, and the default when type is omitted
- badge: status pill, status defaults to inactive
- actions slot: optional right-aligned action buttons
-->
<script setup>
const props = defineProps({
  items: {
    type: Array,
    required: true,
  },
});

const badgeClasses = {
  ok: 'badge-success',
  error: 'badge-danger',
  warning: 'badge-warn',
  inactive: 'badge-dim',
};

function badgeClass(status) {
  return badgeClasses[status] || badgeClasses.inactive;
}
</script>

<template>
  <div class="card status-bar">
    <div class="flex status-bar-values">
      <div v-for="item in props.items" :key="item.title" class="flex status-item">
        <span class="text-dim">{{ item.title }}</span>
        <span v-if="item.type === 'badge'" :class="['badge', badgeClass(item.status)]">{{ item.value }}</span>
        <strong v-else>{{ item.value }}</strong>
      </div>
    </div>
    <div v-if="$slots.actions" class="flex btn-group status-bar-actions">
      <slot name="actions" />
    </div>
  </div>
</template>

<style scoped>
.status-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
  padding: 12px 16px;
  margin-bottom: 16px;
}
.status-bar-values {
  flex: 1;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
}
.status-item {
  gap: 6px;
  align-items: center;
}
.status-item .text-dim {
  font-size: 12px;
}
.status-bar-actions {
  margin-left: auto;
}
@media (max-width: 720px) {
  .status-bar-actions {
    margin-left: 0;
  }
}
</style>
