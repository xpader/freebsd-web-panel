<script setup>
import TopBar from './TopBar.vue';
import SideBar from './SideBar.vue';
import { useRoute } from 'vue-router';
import { computed } from 'vue';
import { groupOfPath, MENU, SETTINGS } from '../../lib/menu.js';

const route = useRoute();
const activeGroup = computed(() => groupOfPath(route.path));
const sidebarItems = computed(() => {
  if (activeGroup.value === 'settings') return SETTINGS;
  const g = MENU.find((m) => m.key === activeGroup.value) || MENU[0];
  return g.items;
});
</script>

<template>
  <div class="topbar">
    <TopBar :active-group="activeGroup" />
  </div>
  <div class="body-wrap">
    <aside class="sidebar">
      <nav class="sidebar-nav">
        <SideBar :items="sidebarItems" />
      </nav>
    </aside>
    <main class="main">
      <router-view />
    </main>
  </div>
</template>
