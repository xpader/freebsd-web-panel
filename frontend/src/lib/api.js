// API client — wraps fetch with auth token and error handling.

import { useAuthStore } from '../stores/auth.js';
import router from '../router/index.js';
import { ui } from '../stores/ui.js';
import i18n from '../i18n/index.js';

function getToken() {
  return sessionStorage.getItem('fwp_token');
}

export function setToken(token) {
  sessionStorage.setItem('fwp_token', token);
}

export function clearToken() {
  sessionStorage.removeItem('fwp_token');
}

export function isLoggedIn() {
  return !!getToken();
}

// Guards against multiple concurrent 401s triggering duplicate dialogs.
let sessionExpiredHandling = false;

async function handleSessionExpired() {
  if (sessionExpiredHandling) return;
  sessionExpiredHandling = true;
  useAuthStore().logout();
  if (router.currentRoute.value.path !== '/login') {
    await router.replace('/login');
  }
  const t = i18n.global.t.bind(i18n.global);
  await ui.showDialog({
    type: 'alert',
    title: t('auth.sessionExpiredTitle'),
    message: t('auth.sessionExpiredMsg'),
  });
  sessionExpiredHandling = false;
}

async function request(method, path, body) {
  const headers = { 'Content-Type': 'application/json' };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;

  const opts = { method, headers };
  if (body !== undefined) opts.body = JSON.stringify(body);

  const res = await fetch(path, opts);
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;

  if (res.status === 401) {
    // On the login page, 401 = wrong credentials — throw so LoginPage can show the error.
    if (router.currentRoute.value.path === '/login') {
      const msg = data?.message || 'unauthenticated';
      throw { status: 401, message: msg, data };
    }
    // Everywhere else: trigger global session-expired flow, then stall forever
    // so the caller's catch never fires (the page is being torn down anyway).
    handleSessionExpired();
    return new Promise(() => {});
  }

  if (!res.ok) {
    const msg = (data && data.message) || `Request failed (${res.status})`;
    throw { status: res.status, message: msg, data };
  }

  return data;
}

export const api = {
  get: (p) => request('GET', p),
  post: (p, b) => request('POST', p, b),
  put: (p, b) => request('PUT', p, b),
  del: (p) => request('DELETE', p),
};

// Raw fetch with auth header for binary downloads/uploads.
export async function authFetch(url, opts = {}) {
  const token = getToken();
  const headers = { ...(opts.headers || {}) };
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const res = await fetch(url, { ...opts, headers });
  if (res.status === 401 && router.currentRoute.value.path !== '/login') {
    handleSessionExpired();
    return new Promise(() => {});
  }
  return res;
}
