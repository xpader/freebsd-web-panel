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

/**
 * Show a countdown dialog with auto-resolve and optional reachability probe.
 *
 * @param {string} title
 * @param {string} message
 * @param {number} expiresAt - Unix timestamp (seconds)
 * @param {number} timeoutSeconds - total countdown duration
 * @param {object} opts
 * @param {string} [opts.rollbackLabel]
 * @param {string} [opts.confirmLabel]
 * @param {string} [opts.warningMessage] - shown in orange bar if the
 *   reachability probe fails
 * @param {string} [opts.probeUrl] - URL to fetch after 1s; omit to skip probe
 */
export function useCountdown() {
  return (title, message, expiresAt, timeoutSeconds, opts = {}) => {
    const total = timeoutSeconds || 60;
    const endTime = expiresAt * 1000;

    const secs = Math.max(0, Math.ceil((endTime - Date.now()) / 1000));
    const config = {
      type: 'countdown',
      title,
      message,
      rollbackLabel: opts.rollbackLabel,
      confirmLabel: opts.confirmLabel,
      secs,
      pct: (secs / total) * 100,
      warning: null,
    };

    const promise = ui.showDialog(config);

    const dialogObj = ui.dialog;

    let timer = null;
    timer = setInterval(() => {
      // If the dialog was closed by other means (user clicked a button),
      // dialog.value is now a different dialog or null — stop the timer.
      if (ui.dialog !== dialogObj) {
        clearInterval(timer);
        return;
      }
      const remaining = Math.max(0, Math.ceil((endTime - Date.now()) / 1000));
      dialogObj.secs = remaining;
      dialogObj.pct = (remaining / total) * 100;
      if (remaining <= 0) {
        clearInterval(timer);
        ui.resolveDialog('rollback');
      }
    }, 500);

    // Clean up the timer when the promise resolves (user clicked a button
    // or countdown expired). Without this, the interval keeps running and
    // may close subsequently opened dialogs.
    promise.then(() => clearInterval(timer));

    if (opts.probeUrl) {
      setTimeout(async () => {
        if (ui.dialog !== dialogObj) return;
        try {
          const controller = new AbortController();
          const to = setTimeout(() => controller.abort(), 5000);
          await fetch(opts.probeUrl, {
            headers: { 'Authorization': `Bearer ${sessionStorage.getItem('fwp_token')}` },
            signal: controller.signal,
          });
          clearTimeout(to);
        } catch (_) {
          if (ui.dialog === dialogObj) {
            dialogObj.warning = opts.warningMessage || null;
          }
        }
      }, 1000);
    }

    return promise;
  };
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
