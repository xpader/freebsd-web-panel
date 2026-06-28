// Confirm dialog — returns a Promise<boolean>.

export function confirmDialog(title, message) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.innerHTML = `
      <div class="modal">
        <h3>${title}</h3>
        <p class="text-dim">${message}</p>
        <div class="modal-actions">
          <button class="btn-secondary" data-act="cancel">取消</button>
          <button class="btn-danger" data-act="ok">确认</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay || e.target.dataset.act === 'cancel') {
        overlay.remove();
        resolve(false);
      } else if (e.target.dataset.act === 'ok') {
        overlay.remove();
        resolve(true);
      }
    });
  });
}
