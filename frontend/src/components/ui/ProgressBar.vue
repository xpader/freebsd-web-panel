<script setup>
import { computed } from 'vue';

const props = defineProps({
  // 百分比 0–100，内部自动 clamp 到 [0,100]
  pct: { type: Number, default: 0 },
  // cpu(蓝) | mem(紫) | swap(橙警告) | auto(按阈值：正常紫，超阈值橙)
  variant: { type: String, default: 'auto' },
  // 尺寸：''(默认) | 'sm'
  size: { type: String, default: '' },
  // auto 模式的告警阈值（百分比）
  threshold: { type: Number, default: 80 },
});

const value = computed(() => {
  const v = Number(props.pct) || 0;
  return Math.max(0, Math.min(100, v));
});

const barClass = computed(() => {
  if (props.variant === 'cpu') return 'bar-cpu';
  if (props.variant === 'mem') return 'bar-mem';
  if (props.variant === 'swap') return 'bar-swap';
  // auto：统一存储类容量条语义——正常紫，超阈值转橙警告
  return value.value > props.threshold ? 'bar-swap' : 'bar-mem';
});
</script>

<template>
  <div class="bar-wrap" :class="{ sm: size === 'sm' }">
    <div class="bar" :class="barClass" :style="{ width: value + '%' }"></div>
  </div>
</template>
