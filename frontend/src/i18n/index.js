// vue-i18n setup — reuses existing translation resources.
//
// The original translations use i18next's `{{placeholder}}` interpolation
// syntax. vue-i18n uses `{placeholder}`. We convert at load time so the
// translation file doesn't need modification.

import { createI18n } from 'vue-i18n';
import { en, zh } from './translations.js';

// Deep-convert {{key}} → {key} in all string values.
function convertMsg(obj) {
  const result = {};
  for (const [key, val] of Object.entries(obj)) {
    if (typeof val === 'string') {
      result[key] = val.replace(/\{\{(\w+)\}\}/g, '{$1}');
    } else if (typeof val === 'object' && val !== null) {
      result[key] = convertMsg(val);
    } else {
      result[key] = val;
    }
  }
  return result;
}

const STORAGE_KEY = 'fwp_lang';

export const LANGUAGES = [
  { code: 'en', label: 'English', flag: '/img/flag-us.svg' },
  { code: 'zh', label: '简体中文', flag: '/img/flag-cn.svg' },
];

function detectInitialLang() {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && LANGUAGES.some((l) => l.code === stored)) return stored;
  const nav = (navigator.language || 'zh').toLowerCase();
  return nav.startsWith('en') ? 'en' : 'zh';
}

const i18n = createI18n({
  legacy: false,
  locale: detectInitialLang(),
  fallbackLocale: 'en',
  messages: {
    en: convertMsg(en),
    zh: convertMsg(zh),
  },
});

export function setLang(code) {
  if (!LANGUAGES.some((l) => l.code === code)) return;
  if (i18n.global.locale.value === code) return;
  i18n.global.locale.value = code;
  localStorage.setItem(STORAGE_KEY, code);
}

export function getLang() {
  return i18n.global.locale.value;
}

export function getLocale() {
  return getLang() === 'zh' ? 'zh-CN' : 'en-US';
}

export function currentLangMeta() {
  return LANGUAGES.find((l) => l.code === getLang()) || LANGUAGES[0];
}

export default i18n;
