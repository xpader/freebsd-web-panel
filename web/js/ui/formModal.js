// Reusable form modal — replaces browser prompt() with styled dialogs.

import { t } from '../i18n/index.js';

/**
 * Show a modal with custom fields. Returns a Promise that resolves with
 * {field: value} on submit, or null on cancel.
 *
 * @param {string} title
 * @param {Array} fields - [{key, label, type, value, placeholder, required, options}]
 * @param {string} submitLabel
 * @returns {Promise<Object|null>}
 */
export function formModal(title, fields, submitLabel) {
  submitLabel = submitLabel || t('common.ok');
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';

    const fieldHtml = fields.map(f => {
      if (f.type === 'select' && f.options) {
        return `
          <div class="field">
            <label>${esc(f.label)}${f.required ? ' <span style="color:var(--danger)">*</span>' : ''}</label>
            <select name="${f.key}" ${f.required ? 'required' : ''}>
              <option value="">${t('common.pleaseSelect')}</option>
              ${f.options.map(o => `<option value="${esc(o.value || o)}" ${f.value === (o.value || o) ? 'selected' : ''}>${esc(o.label || o)}</option>`).join('')}
            </select>
          </div>`;
      }
      if (f.type === 'textarea') {
        return `
          <div class="field">
            <label>${esc(f.label)}</label>
            <textarea name="${f.key}" rows="${f.rows || 3}" placeholder="${esc(f.placeholder || '')}">${esc(f.value || '')}</textarea>
          </div>`;
      }
      const inputType = f.type === 'password' ? 'password' : 'text';
      return `
        <div class="field">
          <label>${esc(f.label)}${f.required ? ' <span style="color:var(--danger)">*</span>' : ''}</label>
          <input type="${inputType}" name="${f.key}" value="${esc(f.value || '')}" placeholder="${esc(f.placeholder || '')}" ${f.required ? 'required' : ''} />
        </div>`;
    }).join('');

    overlay.innerHTML = `
      <div class="modal">
        <h3>${esc(title)}</h3>
        <form id="modal-form">
          ${fieldHtml}
          <div class="modal-actions">
            <button type="button" class="btn-secondary" data-act="cancel">${t('common.cancel')}</button>
            <button type="submit">${esc(submitLabel)}</button>
          </div>
        </form>
      </div>`;

    document.body.appendChild(overlay);

    const close = (result) => {
      overlay.remove();
      resolve(result);
    };

    overlay.addEventListener('click', (e) => {
      if (e.target.dataset.act === 'cancel') close(null);
    });

    overlay.querySelector('#modal-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const formData = new FormData(e.target);
      const result = {};
      for (const f of fields) {
        result[f.key] = formData.get(f.key);
      }
      close(result);
    });

    // Auto-focus first input.
    setTimeout(() => {
      const first = overlay.querySelector('input, select, textarea');
      if (first) first.focus();
    }, 50);
  });
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
