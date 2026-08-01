<script setup>
import { ref, reactive, onMounted, onUnmounted, computed } from 'vue';
import { useRouter, useRoute } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { MENU, SETTINGS, activeChildIndex } from '../../lib/menu.js';
import { LANGUAGES, setLang, currentLangMeta } from '../../i18n/index.js';
import { useAuthStore } from '../../stores/auth.js';
import { preference as themePref, setTheme } from '../../stores/theme.js';
import { api } from '../../lib/api.js';
import { useConfirm } from '../../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const route = useRoute();
const auth = useAuthStore();
const confirm = useConfirm();

defineProps({
  activeGroup: { type: String, default: 'overview' },
});

const openMenu = ref(null); // which right-side dropdown is open: 'lang'|'theme'|'settings'|'user'|null
const curLang = ref(currentLangMeta());
const openKey = ref(null); // which top-nav group's hover submenu is open

const themeOptions = [
  { val: 'auto', icon: 'fa-solid fa-circle-half-stroke', labelKey: 'topbar.themeSystem' },
  { val: 'light', icon: 'fa-solid fa-sun', labelKey: 'topbar.themeLight' },
  { val: 'dark', icon: 'fa-solid fa-moon', labelKey: 'topbar.themeDark' },
];
const themeIcon = computed(() =>
  themeOptions.find((o) => o.val === themePref.value)?.icon || 'fa-solid fa-circle-half-stroke',
);

// Power overlay state
const power = reactive({ visible: false, mode: '', phase: 'waiting' });
let powerTimer = null;
// Toggle a right-side dropdown: opens it (closing any other open menu incl.
// the top-nav hover submenu), or closes it if already open.
function toggleMenu(which) {
  openKey.value = null;
  openMenu.value = openMenu.value === which ? null : which;
}

function switchLang(code) {
  openMenu.value = null;
  setLang(code);
  curLang.value = currentLangMeta();
}

async function doLogout() {
  openMenu.value = null;
  try { await api.post('/api/auth/logout'); } catch {}
  auth.logout();
  router.push('/login');
}

async function probeServer() {
  try {
    const r = await fetch('/api/users/bootstrap', { signal: AbortSignal.timeout(3000) });
    return r.ok;
  } catch { return false; }
}

function startPowerPoll(mode) {
  power.visible = true;
  power.mode = mode;
  power.phase = 'waiting';
  powerTimer = setInterval(async () => {
    const reachable = await probeServer();
    if (!reachable && power.phase === 'waiting') {
      power.phase = mode === 'reboot' ? 'rebooting' : 'done';
    }
    if (mode === 'reboot' && power.phase === 'rebooting' && reachable) {
      clearInterval(powerTimer);
      powerTimer = null;
      location.reload();
      return;
    }
    if (power.phase === 'done' && mode === 'shutdown') {
      clearInterval(powerTimer);
      powerTimer = null;
    }
  }, 2000);
}

function closePowerOverlay() {
  power.visible = false;
  if (powerTimer) { clearInterval(powerTimer); powerTimer = null; }
  if (power.mode === 'reboot' && power.phase === 'done') {
    auth.logout();
    router.push('/login');
  }
}

async function doPower(mode) {
  openMenu.value = null;
  const ok = await confirm(t(`topbar.${mode}`), t(`topbar.${mode}Confirm`));
  if (!ok) return;
  try {
    await api.post(`/api/system/${mode}`);
  } catch {}
  startPowerPoll(mode);
}

function closeOnClick(e) {
  // Clicks inside a dropdown are stopped at the toggle button (.stop) or close
  // the menu themselves; a click reaching the document is "outside" — close all.
  if (!e.target.closest('.settings-menu') && !e.target.closest('.topnav-tab-wrap')) {
    openMenu.value = null;
    openKey.value = null;
  }
}

onMounted(() => {
  document.addEventListener('click', closeOnClick);
});
onUnmounted(() => {
  if (powerTimer) clearInterval(powerTimer);
});
</script>

<template>
  <a class="brand" href="#/dashboard">
    <span class="brand-mark"><i class="fa-solid fa-bolt"></i></span>
    <span class="brand-text">fwp</span>
  </a>
  <nav class="topnav">
    <div
      v-for="g in MENU"
      :key="g.key"
      :class="['topnav-tab-wrap', { open: openKey === g.key }]"
      @mouseenter="openMenu = null; openKey = g.key"
      @mouseleave="openKey = null"
      @click="openKey = null"
    >
      <a
        :href="'#' + g.default"
        :class="['topnav-tab', { active: g.key === activeGroup }]"
      >
        <span class="icon"><i :class="g.icon"></i></span>{{ t(g.labelKey) }}
      </a>
      <div :class="['topnav-submenu', { open: openKey === g.key }]">
        <template v-for="item in g.items" :key="item.path">
          <a
            v-if="!item.children"
            :href="'#' + item.path"
            :class="['submenu-link', { active: route.path === item.path }]"
          >
            <span class="icon"><i :class="item.icon"></i></span>{{ item.labelKey ? t(item.labelKey) : item.label }}
          </a>
          <div v-else class="submenu-group">
            <div :class="['submenu-group-title', { active: activeChildIndex(item, route.path) >= 0 }]">
              <span class="icon"><i :class="item.icon"></i></span>{{ item.labelKey ? t(item.labelKey) : item.label }}
            </div>
            <div class="submenu-children">
              <a
                v-for="(c, ci) in item.children"
                :key="c.path"
                :href="'#' + c.path"
                :class="['submenu-link submenu-sub', { active: activeChildIndex(item, route.path) === ci }]"
              >
                <span class="icon"><i :class="c.icon"></i></span>{{ c.labelKey ? t(c.labelKey) : c.label }}
              </a>
            </div>
          </div>
        </template>
      </div>
    </div>
  </nav>
  <div class="topbar-right">
    <!-- Language switcher -->
    <div :class="['settings-menu', { open: openMenu === 'lang' }]" id="lang-menu">
      <button class="lang-btn" @click.stop="toggleMenu('lang')" :title="t('topbar.language')">
        <img :src="curLang.flag" class="flag-img" :alt="curLang.label">
      </button>
      <div :class="['settings-dropdown', { open: openMenu === 'lang' }]">
        <a
          v-for="l in LANGUAGES"
          :key="l.code"
          href="#"
          :class="['lang-item', { active: l.code === curLang.code }]"
          @click.prevent="switchLang(l.code)"
        >
          <img :src="l.flag" class="flag-img" :alt="l.label">{{ l.label }}
        </a>
      </div>
    </div>

    <!-- Theme switcher -->
    <div :class="['settings-menu', { open: openMenu === 'theme' }]" id="theme-menu">
      <button class="theme-btn" @click.stop="toggleMenu('theme')" :title="t('topbar.theme')">
        <i :class="themeIcon"></i>
      </button>
      <div :class="['settings-dropdown', { open: openMenu === 'theme' }]">
        <a
          v-for="opt in themeOptions"
          :key="opt.val"
          href="#"
          :class="['theme-item', { active: opt.val === themePref }]"
          @click.prevent="setTheme(opt.val); openMenu = null"
        >
          <span class="icon"><i :class="opt.icon"></i></span>{{ t(opt.labelKey) }}
        </a>
      </div>
    </div>

    <!-- Settings dropdown -->
    <div :class="['settings-menu', { open: openMenu === 'settings' }]" id="settings-menu">
      <button
        :class="['settings-btn', { active: activeGroup === 'settings' }]"
        @click.stop="toggleMenu('settings')"
      >
        <span class="icon"><i class="fa-solid fa-gear"></i></span>{{ t('topbar.settings') }}
      </button>
      <div :class="['settings-dropdown', { open: openMenu === 'settings' }]">
        <a
          v-for="s in SETTINGS"
          :key="s.path"
          :href="'#' + s.path"
          @click="openMenu = null"
        >
          <span class="icon"><i :class="s.icon"></i></span>{{ s.labelKey ? t(s.labelKey) : s.label }}
        </a>
        <div class="dropdown-divider"></div>
        <a href="#" @click.prevent="doPower('shutdown')">
          <span class="icon"><i class="fa-solid fa-power-off"></i></span>{{ t('topbar.shutdown') }}
        </a>
        <a href="#" @click.prevent="doPower('reboot')">
          <span class="icon"><i class="fa-solid fa-rotate-right"></i></span>{{ t('topbar.reboot') }}
        </a>
      </div>
    </div>

    <!-- User dropdown -->
    <div :class="['settings-menu', { open: openMenu === 'user' }]" id="user-menu">
      <button class="user-chip" @click.stop="toggleMenu('user')">
        <span class="icon"><i class="fa-solid fa-circle-user"></i></span>
        <span class="user-name">{{ auth.user?.username || '…' }}</span>
      </button>
      <div :class="['settings-dropdown', { open: openMenu === 'user' }]">
        <a href="#" @click.prevent="doLogout">
          <span class="icon"><i class="fa-solid fa-power-off"></i></span>{{ t('topbar.logout') }}
        </a>
      </div>
    </div>
  </div>

  <!-- Power action overlay -->
  <Teleport to="body">
    <div v-if="power.visible" class="power-overlay">
      <div class="power-card">
        <!-- Waiting for server to go down -->
        <template v-if="power.phase === 'waiting'">
          <div class="power-spinner"><i class="fa-solid fa-spinner fa-spin"></i></div>
          <div class="power-text">{{ power.mode === 'shutdown' ? t('topbar.shuttingDown') : t('topbar.rebooting') }}</div>
        </template>
        <!-- Rebooting: server is down, waiting for it to come back -->
        <template v-else-if="power.phase === 'rebooting'">
          <div class="power-spinner"><i class="fa-solid fa-spinner fa-spin"></i></div>
          <div class="power-text">{{ t('topbar.waitingReboot') }}</div>
        </template>
        <!-- Done (shutdown only; reboot auto-reloads) -->
        <template v-else>
          <div class="power-icon">
            <i class="fa-solid fa-power-off"></i>
          </div>
          <div class="power-text">{{ t('topbar.shutdownDone') }}</div>
        </template>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
/* Brand (brand-mark base style stays global — shared with login page) */
.brand {
  display: flex;
  align-items: center;
  gap: 8px;
  text-decoration: none;
  white-space: nowrap;
}
.brand-text {
  font-weight: 800;
  font-size: 17px;
  letter-spacing: -0.02em;
  color: var(--accent);
}
.brand:hover .brand-text { color: var(--accent-hover); }

/* Top-nav hover submenu (drop-down list of a group's items) */
.topnav { display: flex; gap: 4px; flex: 1; }
.topnav-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  border-radius: 6px;
  color: var(--text-dim);
  font-size: 13px;
  white-space: nowrap;
  position: relative;
}
.topnav-tab:hover { background: var(--hover-bg); color: var(--text); }
.topnav-tab.active { color: var(--accent); background: var(--accent-glow); }
.topnav-tab.active::after {
  content: '';
  position: absolute;
  bottom: -10px;
  left: 8px;
  right: 8px;
  height: 3px;
  border-radius: 3px 3px 0 0;
  background: var(--accent);
}
.topnav-tab .icon { font-size: 14px; opacity: 0.85; }
.topnav-tab-wrap.open .topnav-tab:not(.active) { background: var(--hover-bg); color: var(--text); }
.topnav-tab-wrap { position: relative; }
.topnav-submenu::before {
  /* transparent hover bridge covering any sub-pixel gap to the tab */
  content: '';
  position: absolute;
  top: -6px;
  left: 0;
  right: 0;
  height: 6px;
}
.topnav-submenu {
  display: none;
  position: absolute;
  top: 100%;
  left: 0;
  min-width: 190px;
  background: var(--menu-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  -webkit-backdrop-filter: blur(12px) saturate(1.4);
  backdrop-filter: blur(12px) saturate(1.4);
  box-shadow: 0 8px 24px var(--shadow);
  padding: 6px;
  z-index: 30;
}
.topnav-submenu.open { display: block; }
.submenu-link {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border-radius: 6px;
  color: var(--menu-text);
  font-size: 13px;
  white-space: nowrap;
}
.submenu-link:hover { background: var(--hover-bg); color: var(--text); }
.submenu-link.active { color: var(--accent); background: var(--accent-glow); }
.submenu-link .icon { width: 16px; text-align: center; opacity: 0.8; }
.submenu-group:not(:first-child) {
  margin-top: 6px;
}
.submenu-group-title {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--menu-text);
  opacity: 0.9;
}
.submenu-group-title .icon { width: 16px; text-align: center; }
.submenu-group-title.active { color: var(--accent); opacity: 1; }
.submenu-children {
  margin: 2px 0 4px 18px;
  padding-left: 8px;
  border-left: 1px solid var(--border);
}
.submenu-group .submenu-link { padding-left: 4px; }

/* Right-side controls */
.topbar-right { display: flex; align-items: center; gap: 12px; }
.user-chip {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-dim);
  padding: 6px 10px;
  background: transparent;
  border: none;
  border-radius: 6px;
  font-family: inherit;
  cursor: pointer;
  white-space: nowrap;
}
.user-chip:hover { background: var(--hover-bg); color: var(--text); }
.user-chip .icon { font-size: 14px; opacity: 0.85; }
#user-menu.open .user-chip { background: var(--hover-bg); color: var(--text); }

/* Settings dropdown button (settings-menu / settings-dropdown container styles
   stay global — shared with login page language/theme switchers) */
.settings-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  background: transparent;
  border: none;
  border-radius: 6px;
  color: var(--text-dim);
  font-size: 13px;
  font-family: inherit;
  cursor: pointer;
  white-space: nowrap;
}
.settings-btn:hover { background: var(--hover-bg); color: var(--text); }
.settings-btn.active { color: var(--accent); }
.settings-menu.open .settings-btn { background: var(--hover-bg); color: var(--text); }
.settings-btn .icon { font-size: 14px; }
.dropdown-divider { height: 1px; background: var(--border); margin: 4px 0; }

/* Power action overlay (Teleported to body; scoped attrs travel with it) */
.power-overlay {
  position: fixed; inset: 0; z-index: 200;
  background: var(--modal-overlay);
  display: flex; align-items: center; justify-content: center;
}
.power-card {
  background: var(--bg-elev); border: 1px solid var(--border); border-radius: 12px;
  padding: 40px 48px; text-align: center; min-width: 280px;
}
.power-spinner { font-size: 36px; color: var(--accent); margin-bottom: 16px; }
.power-icon { font-size: 48px; color: var(--accent); margin-bottom: 16px; }
.power-text { font-size: 16px; color: var(--text); margin-bottom: 20px; }
.power-btn {
  background: var(--accent); color: #fff; border: none; border-radius: 6px;
  padding: 8px 24px; cursor: pointer; font-size: 14px;
}
.power-btn:hover { opacity: 0.9; }
</style>
