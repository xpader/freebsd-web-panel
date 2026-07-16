// Dialog composables — imperative API matching the original vanilla JS pattern.
// Each returns a Promise, just like the original confirmDialog/alertDialog/formModal.

import { ui } from '../stores/ui.js';

export function useToast() {
  return {
    toast: (message, type, duration) => ui.showToast(message, type, duration),
  };
}

export function useConfirm() {
  return (title, message, options) =>
    ui.showDialog({ type: 'confirm', title, message, options });
}

export function useAlert() {
  return (title, message) =>
    ui.showDialog({ type: 'alert', title, message });
}

export function useCodePreview() {
  return (title, content) =>
    ui.showDialog({ type: 'code', title, content });
}

export function useFormModal() {
  return (title, fields, opts = {}) =>
    ui.showDialog({
      type: 'form',
      title,
      fields,
      submitLabel: opts.submitLabel,
      errorMessage: opts.errorMessage,
      submitHandler: opts.submitHandler,
    });
}
