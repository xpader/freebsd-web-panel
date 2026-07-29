<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useAuthStore } from '../stores/auth.js';
import { useToast, useAlert } from '../composables/useDialog.js';
import { LANGUAGES, setLang, currentLangMeta } from '../i18n/index.js';
import { preference as themePref, setTheme } from '../stores/theme.js';

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();
const toast = useToast();
const alert = useAlert();

const username = ref('');
const password = ref('');
const loading = ref(false);

const langOpen = ref(false);
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

function toggleLang() { langOpen.value = !langOpen.value; themeOpen.value = false; }
function toggleTheme() { themeOpen.value = !themeOpen.value; langOpen.value = false; }

function switchLang(code) {
  langOpen.value = false;
  setLang(code);
  curLang.value = currentLangMeta();
}

function closeOnClick(e) {
  if (!e.target.closest('#login-lang-menu')) langOpen.value = false;
  if (!e.target.closest('#login-theme-menu')) themeOpen.value = false;
}

onMounted(() => {
  document.addEventListener('click', closeOnClick);
});
onUnmounted(() => {
  document.removeEventListener('click', closeOnClick);
});

async function onSubmit() {
  loading.value = true;
  try {
    const res = await api.post('/api/auth/login', {
      username: username.value,
      password: password.value,
    });
    auth.login(res.token);
    await auth.fetchUser();
    toast.toast(t('auth.welcome', { name: res.user.username }));
    router.push('/dashboard');
  } catch (err) {
    let msg;
    if (err.status === 429) {
      const kind = err.data?.error;
      if (kind === 'ip_banned') {
        msg = t('auth.ipBanned');
      } else {
        msg = t('auth.tooManyAttempts');
      }
    } else if (err.status === 401) {
      msg = t('auth.invalidCredentials');
    } else {
      msg = err.message || t('auth.loginFailed');
    }
    await alert(t('auth.loginFailed'), msg);
    loading.value = false;
  }
}
</script>

<template>
  <div class="login-wrap">
    <div class="login-card">
      <div class="login-brand">
        <span class="brand-mark"><i class="fa-solid fa-bolt"></i></span>
        <h1>FreeBSD Web Panel</h1>
        <p class="subtitle">{{ t('auth.slogan') }}</p>
      </div>
      <form @submit.prevent="onSubmit">
        <div class="field">
          <label>{{ t('auth.username') }}</label>
          <input type="text" v-model="username" autocomplete="username" required />
        </div>
        <div class="field">
          <label>{{ t('auth.password') }}</label>
          <input type="password" v-model="password" autocomplete="current-password" required />
        </div>
        <button type="submit" :disabled="loading" style="width:100%;justify-content:center;">
          {{ loading ? t('auth.loggingIn') : t('auth.login') }}
        </button>
      </form>
    </div>

    <div class="login-footer">
      <div class="login-footer-item">
        <span class="login-footer-label">{{ t('topbar.language') }}</span>
        <div :class="['settings-menu', { open: langOpen }]" id="login-lang-menu">
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
      </div>

      <div class="login-footer-item">
        <span class="login-footer-label">{{ t('topbar.theme') }}</span>
        <div :class="['settings-menu', { open: themeOpen }]" id="login-theme-menu">
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
      </div>
    </div>
  </div>
</template>
