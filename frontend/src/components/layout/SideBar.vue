<script setup>
import { computed } from 'vue';
import { useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { activeChildIndex } from '../../lib/menu.js';

const route = useRoute();
const { t } = useI18n();

const props = defineProps({
  items: { type: Array, default: () => [] },
});

const itemStates = computed(() =>
  props.items.map((item) => {
    if (item.children) {
      const idx = activeChildIndex(item, route.path);
      return { expanded: idx >= 0, activeChildIdx: idx };
    }
    return { active: route.path === item.path };
  })
);
</script>

<template>
  <template v-for="(item, i) in items" :key="item.path">
    <!-- Collapsible group -->
    <div v-if="item.children" :class="['sub-group', { expanded: itemStates[i].expanded }]">
      <div class="sub-group-header" @click="$router.push(item.children[0].path)">
        <span class="icon"><i :class="item.icon"></i></span>{{ item.labelKey ? t(item.labelKey) : item.label }}
        <span class="sub-arrow">
          <i :class="itemStates[i].expanded ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
        </span>
      </div>
      <div class="sub-items">
        <a
          v-for="(c, ci) in item.children"
          :key="c.path"
          :href="'#' + c.path"
          :class="['sub-item', { active: itemStates[i].activeChildIdx === ci }]"
        >
          <span class="icon"><i :class="c.icon"></i></span>{{ c.labelKey ? t(c.labelKey) : c.label }}
        </a>
      </div>
    </div>

    <!-- Direct link -->
    <a
      v-else
      :href="'#' + item.path"
      :class="{ active: itemStates[i].active }"
    >
      <span class="icon"><i :class="item.icon"></i></span>{{ item.labelKey ? t(item.labelKey) : item.label }}
    </a>
  </template>
</template>
