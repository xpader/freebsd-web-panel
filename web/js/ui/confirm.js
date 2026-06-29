// Confirm dialog — returns a Promise<boolean> or Promise<{confirmed, options}>.

import { t } from '../i18n/index.js';

/**
 * @param {string} title
 * @param {string} message
 * @param {Array} [options] - checkbox options: [{key, label, checked}]
 *   When provided, resolves with {confirmed, ...checkboxes} instead of boolean.
 */
export function confirmDialog(title, message, options) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    const optHtml = options && options.length
      ? options.map(o => `
        <label class="confirm-opt" style="display:flex;align-items:center;gap:8px;margin-top:12px;font-size:13px;cursor:pointer;">
          <input type="checkbox" data-opt="${o.key}" ${o.checked ? 'checked' : ''} />
          <span>${o.label}</span>
        </label>`).join('')
      : '';
    overlay.innerHTML = `
      <div class="modal">
        <h3>${title}</h3>
        <p class="text-dim">${message}</p>
        ${optHtml}
        <div class="modal-actions">
          <button class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
          <button class="btn-danger" data-act="ok">${t('common.confirm')}</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay || e.target.dataset.act === 'cancel') {
        overlay.remove();
        resolve(false);
      } else if (e.target.dataset.act === 'ok') {
        if (options && options.length) {
          const result = { confirmed: true };
          options.forEach((o) => {
            const cb = overlay.querySelector(`[data-opt="${o.key}"]`);
            result[o.key] = cb ? cb.checked : false;
          });
          overlay.remove();
          resolve(result);
        } else {
          overlay.remove();
          resolve(true);
        }
      }
    });
  });
}
