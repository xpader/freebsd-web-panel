// UI store — toast notifications + imperative dialogs (confirm, alert, formModal).
//
// A single dialog can be active at a time. DialogHost renders it and
// resolveDialog clears it, resolving the caller's Promise.

import { ref, reactive } from 'vue';

const toasts = ref([]);
const dialog = ref(null);

let toastSeq = 0;
let dialogSeq = 0;

function showToast(message, type = 'success', duration = 3000) {
  const id = ++toastSeq;
  toasts.value.push({ id, message, type });
  setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }, duration);
}

function dismissToast(id) {
  toasts.value = toasts.value.filter((t) => t.id !== id);
}

function showDialog(cfg) {
  return new Promise((resolve) => {
    const id = ++dialogSeq;
    dialog.value = { ...cfg, id, resolve };
  });
}

function resolveDialog(value) {
  if (dialog.value) {
    const d = dialog.value;
    dialog.value = null;
    d.resolve(value);
  }
}

export const ui = reactive({
  toasts,
  dialog,
  showToast,
  dismissToast,
  showDialog,
  resolveDialog,
});
