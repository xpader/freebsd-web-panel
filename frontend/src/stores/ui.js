// UI store — toast notifications + imperative dialogs (confirm, alert, formModal).
//
// Dialogs use a stack: multiple dialogs can coexist (e.g. an alert on top
// of a form). DialogHost renders all stacked dialogs. resolveDialog pops
// the topmost dialog and resolves its Promise.

import { defineStore } from 'pinia';
import { ref, computed } from 'vue';

let dialogSeq = 0;

export const useUiStore = defineStore('ui', () => {
  const toasts = ref([]);
  const dialogs = ref([]);

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

  function showDialog(cfg) {
    return new Promise((resolve) => {
      const id = ++dialogSeq;
      dialogs.value.push({ ...cfg, id, resolve });
    });
  }

  function resolveDialog(value) {
    if (dialogs.value.length) {
      const d = dialogs.value[dialogs.value.length - 1];
      dialogs.value.pop();
      d.resolve(value);
    }
  }

  const dialog = computed(() => dialogs.value[dialogs.value.length - 1] || null);

  return { toasts, dialogs, dialog, showToast, dismissToast, showDialog, resolveDialog };
});
