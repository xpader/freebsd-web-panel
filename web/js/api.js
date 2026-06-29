// API client — wraps fetch with auth token and error handling.

import { t } from './i18n/index.js';

const BASE = ''; // same origin

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

  const res = await fetch(BASE + path, opts);
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;

  if (res.status === 401) {
    clearToken();
    if (location.hash !== '#/login') location.hash = '#/login';
    throw { status: 401, message: t('common.unauthenticated'), data };
  }

  if (!res.ok) {
    const msg = (data && data.message) || t('common.requestFailed', { status: res.status });
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
