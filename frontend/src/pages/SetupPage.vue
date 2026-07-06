<script setup>
import { ref } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { useAuthStore } from '../stores/auth.js';
import { useToast, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();
const toast = useToast();
const alert = useAlert();

const username = ref('');
const password = ref('');
const password2 = ref('');
const loading = ref(false);

async function onSubmit() {
  if (password.value !== password2.value) {
    await alert(t('auth.passwordMismatch'), t('auth.passwordMismatch'));
    return;
  }
  loading.value = true;
  try {
    const res = await api.post('/api/users/bootstrap', {
      username: username.value,
      password: password.value,
    });
    auth.invalidateSetup();
    toast.toast(t('auth.setupDone', { name: res.username }));
    router.push('/login');
  } catch (err) {
    await alert(t('auth.setupFailed'), err.message || t('auth.setupFailed'));
    loading.value = false;
  }
}
</script>

<template>
  <div class="login-wrap">
    <div class="login-card">
      <h1>{{ t('auth.setupTitle') }}</h1>
      <p class="subtitle">{{ t('auth.setupSubtitle') }}</p>
      <form @submit.prevent="onSubmit">
        <div class="field">
          <label>{{ t('auth.username') }}</label>
          <input type="text" v-model="username" required :placeholder="t('auth.usernamePlaceholder')" />
        </div>
        <div class="field">
          <label>{{ t('auth.passwordMin') }}</label>
          <input type="password" v-model="password" required minlength="6" />
        </div>
        <div class="field">
          <label>{{ t('auth.confirmPassword') }}</label>
          <input type="password" v-model="password2" required minlength="6" />
        </div>
        <button type="submit" :disabled="loading" style="width:100%;justify-content:center;">
          {{ loading ? t('auth.creating') : t('auth.createAccount') }}
        </button>
      </form>
    </div>
  </div>
</template>
