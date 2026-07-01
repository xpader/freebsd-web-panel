// i18n wrapper around the vendored i18next UMD build.
//
// i18next is loaded lazily as a UMD global (same pattern as Chart.js),
// then initialized once with the en/zh resources. Pages call `t()` and
// `getLocale()` synchronously after `initI18n()` has resolved in main.js.
//
// Language is persisted in localStorage ('fwp_lang') and falls back to the
// browser language on first visit. Switching language dispatches a
// 'fwp:langchange' window event so the router can re-render the page.

import { en, zh } from './translations.js';

const STORAGE_KEY = 'fwp_lang';

// Supported languages — SVG flag files (emoji flags don't render on Windows).
export const LANGUAGES = [
  { code: 'en', label: 'English', flag: '/img/flag-us.svg' },
  { code: 'zh', label: '简体中文', flag: '/img/flag-cn.svg' },
];

let initPromise = null;

// Inject the vendored UMD bundle via a <script> tag (caches on window.i18next).
function loadI18next() {
  if (window.i18next) return Promise.resolve(window.i18next);
  return new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = '/vendor/i18next.min.js';
    s.onload = () => resolve(window.i18next);
    s.onerror = () => reject(new Error('failed to load /vendor/i18next.min.js'));
    document.head.appendChild(s);
  });
}

function detectInitialLang() {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && LANGUAGES.some((l) => l.code === stored)) return stored;
  // Project is zh-first; default to Chinese unless the browser is English.
  const nav = (navigator.language || 'zh').toLowerCase();
  return nav.startsWith('en') ? 'en' : 'zh';
}

// Initialize i18next once. Must be awaited before the first render so that
// `t()` is synchronous everywhere else.
export function initI18n() {
  if (initPromise) return initPromise;
  initPromise = loadI18next().then((i18n) => {
    i18n.init({
      lng: detectInitialLang(),
      fallbackLng: 'en',
      resources: {
        en: { translation: en },
        zh: { translation: zh },
      },
      returnEmptyString: false,
      interpolation: { escapeValue: false },
    });
    return i18n;
  });
  return initPromise;
}

// Translate a key, with optional vars for {{placeholder}} interpolation.
export function t(key, vars) {
  const i18n = window.i18next;
  if (!i18n || !i18n.isInitialized) return key;
  return vars ? i18n.t(key, vars) : i18n.t(key);
}

export function getLang() {
  const i18n = window.i18next;
  return (i18n && i18n.isInitialized && i18n.language) || detectInitialLang();
}

export function setLang(code) {
  if (!LANGUAGES.some((l) => l.code === code)) return;
  const i18n = window.i18next;
  if (!i18n || !i18n.language || i18n.language === code) return;
  i18n.changeLanguage(code);
  localStorage.setItem(STORAGE_KEY, code);
  window.dispatchEvent(new CustomEvent('fwp:langchange'));
}

// Locale string for Date.toLocaleString() etc.
export function getLocale() {
  return getLang() === 'zh' ? 'zh-CN' : 'en-US';
}

// Active language metadata { code, label, flag }.
export function currentLangMeta() {
  return LANGUAGES.find((l) => l.code === getLang()) || LANGUAGES[0];
}
