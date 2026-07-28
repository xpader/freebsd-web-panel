<script setup>
import { ref, onMounted, computed } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { MENU, SETTINGS } from '../../lib/menu.js';
import { LANGUAGES, setLang, currentLangMeta, getLang } from '../../i18n/index.js';
import { useAuthStore } from '../../stores/auth.js';
import { preference as themePref, setTheme } from '../../stores/theme.js';
import { api } from '../../lib/api.js';

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();

defineProps({
  activeGroup: { type: String, default: 'overview' },
});

const langOpen = ref(false);
const settingsOpen = ref(false);
const userOpen = ref(false);
const themeOpen = ref(false);
const curLang = ref(currentLangMeta());

const themeOptions = [
  { val: 'auto', icon: 'fa-solid fa-circle-half-stroke', labelKey: 'topbar.themeSystem' },
  { val: 'light', icon: 'fa-solid fa-sun', labelKey: 'topbar.themeLight' },
  { val: 'dark', icon: 'fa-solid fa-moon', labelKey: 'topbar.themeDark' },
];
const themeIcon = computed(() =>
  themeOptions.find((o) => o.val === themePref.value)?.icon || 'fa-solid fa-circle-half-stroke',
);

function toggleLang() { langOpen.value = !langOpen.value; }
function toggleSettings() { settingsOpen.value = !settingsOpen.value; }
function toggleUser() { userOpen.value = !userOpen.value; }
function toggleTheme() { themeOpen.value = !themeOpen.value; }

function switchLang(code) {
  langOpen.value = false;
  setLang(code);
  curLang.value = currentLangMeta();
}

async function doLogout() {
  userOpen.value = false;
  try { await api.post('/api/auth/logout'); } catch {}
  auth.logout();
  router.push('/login');
}

function closeOnClick(e) {
  if (!e.target.closest('#lang-menu')) langOpen.value = false;
  if (!e.target.closest('#theme-menu')) themeOpen.value = false;
  if (!e.target.closest('#settings-menu')) settingsOpen.value = false;
  if (!e.target.closest('#user-menu')) userOpen.value = false;
}

onMounted(() => {
  document.addEventListener('click', closeOnClick);
});
</script>

<template>
  <a class="brand" href="#/dashboard">
    <span class="brand-mark"><i class="fa-solid fa-bolt"></i></span>
    <span class="brand-text">fwp</span>
  </a>
  <nav class="topnav">
    <a
      v-for="g in MENU"
      :key="g.key"
      :href="'#' + g.default"
      :class="['topnav-tab', { active: g.key === activeGroup }]"
    >
      <span class="icon"><i :class="g.icon"></i></span>{{ t(g.labelKey) }}
    </a>
  </nav>
  <div class="topbar-right">
    <!-- Language switcher -->
    <div :class="['settings-menu', { open: langOpen }]" id="lang-menu">
      <button class="lang-btn" @click.stop="toggleLang" :title="t('topbar.language')">
        <img :src="curLang.flag" class="flag-img" :alt="curLang.label">
      </button>
      <div :class="['settings-dropdown', { open: langOpen }]">
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
    <div :class="['settings-menu', { open: themeOpen }]" id="theme-menu">
      <button class="theme-btn" @click.stop="toggleTheme" :title="t('topbar.theme')">
        <i :class="themeIcon"></i>
      </button>
      <div :class="['settings-dropdown', { open: themeOpen }]">
        <a
          v-for="opt in themeOptions"
          :key="opt.val"
          href="#"
          :class="['theme-item', { active: opt.val === themePref }]"
          @click.prevent="setTheme(opt.val); themeOpen = false"
        >
          <span class="icon"><i :class="opt.icon"></i></span>{{ t(opt.labelKey) }}
        </a>
      </div>
    </div>

    <!-- Settings dropdown -->
    <div :class="['settings-menu', { open: settingsOpen }]" id="settings-menu">
      <button
        :class="['settings-btn', { active: activeGroup === 'settings' }]"
        @click.stop="toggleSettings"
      >
        <span class="icon"><i class="fa-solid fa-gear"></i></span>{{ t('topbar.settings') }}
      </button>
      <div :class="['settings-dropdown', { open: settingsOpen }]">
        <a
          v-for="s in SETTINGS"
          :key="s.path"
          :href="'#' + s.path"
          @click="settingsOpen = false"
        >
          <span class="icon"><i :class="s.icon"></i></span>{{ s.labelKey ? t(s.labelKey) : s.label }}
        </a>
      </div>
    </div>

    <!-- User dropdown -->
    <div :class="['settings-menu', { open: userOpen }]" id="user-menu">
      <button class="user-chip" @click.stop="toggleUser">
        <span class="icon"><i class="fa-solid fa-circle-user"></i></span>
        <span class="user-name">{{ auth.user?.username || '…' }}</span>
      </button>
      <div :class="['settings-dropdown', { open: userOpen }]">
        <a href="#" @click.prevent="doLogout">
          <span class="icon"><i class="fa-solid fa-power-off"></i></span>{{ t('topbar.logout') }}
        </a>
      </div>
    </div>
  </div>
</template>
