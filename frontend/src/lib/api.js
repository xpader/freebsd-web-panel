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

function handleSessionExpired() {
  if (sessionExpiredHandling) return;
  sessionExpiredHandling = true;
  clearToken();
  const t = i18n.global.t.bind(i18n.global);
  ui.showDialog({
    type: 'alert',
    title: t('auth.sessionExpiredTitle'),
    message: t('auth.sessionExpiredMsg'),
  }).then(() => {
    sessionExpiredHandling = false;
    if (router.currentRoute.value.path !== '/login') {
      router.push('/login');
    }
  });
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
    if (router.currentRoute.value.path !== '/login') {
      handleSessionExpired();
    }
    // On the login page, prefer the backend message (e.g. "invalid credentials").
    const msg = (router.currentRoute.value.path === '/login' && data?.message)
      ? data.message
      : 'unauthenticated';
    throw { status: 401, message: msg, data };
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
  }
  return res;
}
