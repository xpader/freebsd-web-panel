// Dialog composables — imperative API matching the original vanilla JS pattern.
// Each returns a Promise, just like the original confirmDialog/alertDialog/formModal.

import { useUiStore } from '../stores/ui.js';

export function useToast() {
  const ui = useUiStore();
  return {
    toast: (message, type, duration) => ui.showToast(message, type, duration),
  };
}

export function useConfirm() {
  const ui = useUiStore();
  return (title, message, options) =>
    ui.showDialog({ type: 'confirm', title, message, options });
}

export function useAlert() {
  const ui = useUiStore();
  return (title, message) =>
    ui.showDialog({ type: 'alert', title, message });
}

export function useFormModal() {
  const ui = useUiStore();
  return (title, fields, submitLabel) =>
    ui.showDialog({ type: 'form', title, fields, submitLabel });
}
