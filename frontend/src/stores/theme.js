// Theme store — manages color theme preference (auto / light / dark).
//
// 'auto' follows the OS prefers-color-scheme media query.
// The effective theme is applied as data-theme attribute on <html>.

import { ref, watch } from 'vue';

const STORAGE_KEY = 'fwp_theme';
const VALID = ['auto', 'light', 'dark'];

let mqListener = null;

function readStored() {
  const v = localStorage.getItem(STORAGE_KEY);
  return VALID.includes(v) ? v : 'auto';
}

function systemDark() {
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function resolve(pref) {
  if (pref === 'auto') return systemDark() ? 'dark' : 'light';
  return pref;
}

function apply(effective) {
  document.documentElement.dataset.theme = effective;
}

const preference = ref(readStored());
const effective = ref(resolve(preference.value));

apply(effective.value);

function setupMqListener() {
  if (mqListener) return;
  const mq = window.matchMedia('(prefers-color-scheme: dark)');
  const handler = (e) => {
    if (preference.value === 'auto') {
      effective.value = e.matches ? 'dark' : 'light';
    }
  };
  mq.addEventListener('change', handler);
  mqListener = { mq, handler };
}

watch(preference, (val) => {
  localStorage.setItem(STORAGE_KEY, val);
  effective.value = resolve(val);
});

watch(effective, (val) => apply(val), { immediate: true });

setupMqListener();

export { preference, effective };

export function useTheme() {
  return { preference, effective };
}

export function setTheme(pref) {
  preference.value = pref;
}
