// API client — wraps fetch with auth token and error handling.

import { useAuthStore } from '../stores/auth.js';
import router from '../router/index.js';

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
    clearToken();
    if (router.currentRoute.value.path !== '/login') {
      router.push('/login');
    }
    throw { status: 401, message: 'unauthenticated', data };
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
export function authFetch(url, opts = {}) {
  const token = getToken();
  const headers = { ...(opts.headers || {}) };
  if (token) headers['Authorization'] = `Bearer ${token}`;
  return fetch(url, { ...opts, headers });
}
