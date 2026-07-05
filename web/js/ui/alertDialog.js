// Alert dialog — displays an important message (e.g. errors) that the user must see.
// Returns a Promise<void> that resolves when dismissed.

import { t } from '../i18n/index.js';

export function alertDialog(title, message) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.innerHTML = `
      <div class="modal">
        <h3>${esc(title)}</h3>
        <p class="text-dim">${typeof message === 'string' ? esc(message) : message}</p>
        <div class="modal-actions">
          <button class="btn-secondary" data-act="ok">${t('common.ok')}</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);
    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'ok') {
        overlay.remove();
        resolve();
      }
    });
  });
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
