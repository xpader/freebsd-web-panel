// UI store — toast notifications + imperative dialogs (confirm, alert, formModal).
//
// Dialogs are pushed to a reactive queue. A single DialogHost component
// renders the active dialog and resolves the caller's Promise on dismissal.

import { defineStore } from 'pinia';
import { ref } from 'vue';

let dialogId = 0;

export const useUiStore = defineStore('ui', () => {
  const toasts = ref([]);
  const dialog = ref(null); // single active dialog at a time

  let toastSeq = 0;

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

  /**
   * Show a dialog and return a Promise.
   * @param {object} cfg - { type: 'confirm'|'alert'|'form', ... }
   */
  function showDialog(cfg) {
    return new Promise((resolve) => {
      const id = ++dialogId;
      dialog.value = { ...cfg, id, resolve };
    });
  }

  function resolveDialog(value) {
    if (dialog.value) {
      dialog.value.resolve(value);
      dialog.value = null;
    }
  }

  return { toasts, dialog, showToast, dismissToast, showDialog, resolveDialog };
});
