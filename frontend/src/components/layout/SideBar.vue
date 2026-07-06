<script setup>
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';

const route = useRoute();
const { t } = useI18n();

defineProps({
  items: { type: Array, default: () => [] },
});

function hasActiveChild(item) {
  if (!item.children) return false;
  return item.children.some((c) => route.path === c.path);
}
</script>

<template>
  <template v-for="item in items" :key="item.path">
    <!-- Collapsible group -->
    <div v-if="item.children" :class="['sub-group', { expanded: hasActiveChild(item) }]">
      <div class="sub-group-header" @click="$router.push(item.children[0].path)">
        <span class="icon"><i :class="item.icon"></i></span>{{ t(item.labelKey) }}
        <span class="sub-arrow">
          <i :class="hasActiveChild(item) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
        </span>
      </div>
      <div class="sub-items">
        <a
          v-for="c in item.children"
          :key="c.path"
          :href="'#' + c.path"
          :class="['sub-item', { active: route.path === c.path }]"
        >
          <span class="icon"><i :class="c.icon"></i></span>{{ t(c.labelKey) }}
        </a>
      </div>
    </div>

    <!-- Direct link -->
    <a
      v-else
      :href="'#' + item.path"
      :class="{ active: route.path === item.path }"
    >
      <span class="icon"><i :class="item.icon"></i></span>{{ t(item.labelKey) }}
    </a>
  </template>
</template>
