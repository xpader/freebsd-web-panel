<script setup>
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { MENU, SETTINGS } from '../../lib/menu.js';
import { LANGUAGES, setLang, currentLangMeta, getLang } from '../../i18n/index.js';
import { useAuthStore } from '../../stores/auth.js';
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
const curLang = ref(currentLangMeta());

function toggleLang() { langOpen.value = !langOpen.value; }
function toggleSettings() { settingsOpen.value = !settingsOpen.value; }
function toggleUser() { userOpen.value = !userOpen.value; }

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
  if (!e.target.closest('#settings-menu')) settingsOpen.value = false;
  if (!e.target.closest('#user-menu')) userOpen.value = false;
}

onMounted(() => {
  document.addEventListener('click', closeOnClick);
});
</script>

<template>
  <div class="topbar-brand">FreeBSD Web Panel</div>
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
