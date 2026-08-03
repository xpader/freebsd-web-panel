<script setup>
import TopBar from './TopBar.vue';
import SideBar from './SideBar.vue';
import { useRoute } from 'vue-router';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { groupOfPath, MENU, SETTINGS } from '../../lib/menu.js';

const route = useRoute();
const { t } = useI18n();
const activeGroup = computed(() => groupOfPath(route.path));
const sidebarItems = computed(() => {
  if (activeGroup.value === 'settings') return SETTINGS;
  const g = MENU.find((m) => m.key === activeGroup.value) || MENU[0];
  return g.items;
});
const standalone = computed(() => route.query.standalone === '1');

const sidebarCollapsed = ref(localStorage.getItem('fwp_sidebar_collapsed') === '1');
function toggleSidebar() {
  sidebarCollapsed.value = !sidebarCollapsed.value;
  localStorage.setItem('fwp_sidebar_collapsed', sidebarCollapsed.value ? '1' : '0');
}
</script>

<template>
  <main v-if="standalone" class="main main-standalone">
    <router-view />
  </main>
  <template v-else>
    <div class="topbar">
      <TopBar :active-group="activeGroup" />
    </div>
    <div class="body-wrap" :class="{ 'sidebar-collapsed': sidebarCollapsed }">
      <aside class="sidebar">
        <nav class="sidebar-nav">
          <SideBar :items="sidebarItems" />
        </nav>
      </aside>
      <button
        class="sidebar-toggle"
        @click="toggleSidebar"
        :title="sidebarCollapsed ? t('common.expandSidebar') : t('common.collapseSidebar')"
      >
        <i :class="sidebarCollapsed ? 'fa-solid fa-chevron-right' : 'fa-solid fa-chevron-left'"></i>
      </button>
      <main class="main">
        <router-view />
      </main>
    </div>
  </template>
</template>

<style scoped>
/* Collapsed sidebar: shrink to zero width, hide content */
.sidebar-collapsed .sidebar {
  width: 0;
  overflow: hidden;
  border-right-color: transparent;
}

/* Collapse tab — fixed to viewport, trapezoid shape via ::before.
   The button itself is a full rectangle (no clip-path) so the entire
   14×60px area is always clickable; the trapezoid is purely visual. */
.sidebar-toggle {
  position: fixed;
  top: calc(50% + 26px);
  margin-top: -30px;
  left: calc(var(--sidebar-w) - 14px);
  z-index: 10;
  width: 14px;
  height: 60px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-dim);
  font-size: 11px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: left 0.2s ease, color 0.15s;
}
.sidebar-toggle:hover { color: var(--text); background: transparent; }
.sidebar-toggle::before {
  content: '';
  position: absolute;
  inset: 0;
  background: var(--bg-elev2);
  clip-path: polygon(0 25%, 100% 0, 100% 100%, 0 75%);
  transition: clip-path 0.2s ease, background 0.15s;
  pointer-events: none;
}
.sidebar-toggle:hover::before { background: var(--btn-sec-hover); }
.sidebar-toggle i { position: relative; z-index: 1; }
.sidebar-collapsed .sidebar-toggle { left: 0; }
.sidebar-collapsed .sidebar-toggle::before {
  clip-path: polygon(0 0, 100% 25%, 100% 75%, 0 100%);
}
</style>
