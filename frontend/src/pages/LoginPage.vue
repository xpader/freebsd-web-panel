<script setup>
import { ref } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api, setToken } from '../lib/api.js';
import { useAuthStore } from '../stores/auth.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();
const toast = useToast();
const alert = useAlert();

const username = ref('');
const password = ref('');
const loading = ref(false);

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
    await alert(t('auth.loginFailed'), err.message || t('auth.loginFailed'));
    loading.value = false;
  }
}
</script>

<template>
  <div class="login-wrap">
    <div class="login-card">
      <h1>FreeBSD Web Panel</h1>
      <p class="subtitle">{{ t('auth.loginSubtitle') }}</p>
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
  </div>
</template>
