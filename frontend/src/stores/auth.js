// Auth store — token management, current user, bootstrap status.

import { defineStore } from 'pinia';
import { ref } from 'vue';
import { api, setToken, clearToken } from '../lib/api.js';

export const useAuthStore = defineStore('auth', () => {
  const token = ref(sessionStorage.getItem('fwp_token') || '');
  const user = ref(null);
  const needsSetup = ref(null); // null = unknown, boolean = resolved
  let setupReq = null;

  function login(tok) {
    token.value = tok;
    setToken(tok);
  }

  function logout() {
    token.value = '';
    user.value = null;
    clearToken();
  }

  function isLoggedIn() {
    return !!token.value;
  }

  async function fetchUser() {
    if (!token.value) return null;
    if (user.value) return user.value;
    try {
      user.value = await api.get('/api/auth/me');
      return user.value;
    } catch {
      return null;
    }
  }

  async function resolveNeedsSetup() {
    if (needsSetup.value !== null) return needsSetup.value;
    if (!setupReq) {
      setupReq = api
        .get('/api/users/bootstrap')
        .then((s) => { needsSetup.value = !!s.needs_setup; return needsSetup.value; })
        .catch(() => { needsSetup.value = false; return false; })
        .finally(() => { setupReq = null; });
    }
    return setupReq;
  }

  function invalidateSetup() {
    needsSetup.value = false;
  }

  return {
    token, user, needsSetup,
    login, logout, isLoggedIn, fetchUser,
    resolveNeedsSetup, invalidateSetup,
  };
});
