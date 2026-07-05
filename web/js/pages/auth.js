// Login page + first-run setup.

import { api, setToken } from '../api.js';
import { invalidateSetup } from '../router.js';
import { toast } from '../ui/toast.js';
import { alertDialog } from '../ui/alertDialog.js';
import { t } from '../i18n/index.js';

export async function renderLogin(app) {
  app.innerHTML = `
    <div class="login-wrap">
      <div class="login-card">
        <h1>FreeBSD Web Panel</h1>
        <p class="subtitle">${t('auth.loginSubtitle')}</p>
        <form id="login-form">
          <div class="field">
            <label>${t('auth.username')}</label>
            <input type="text" name="username" autocomplete="username" required />
          </div>
          <div class="field">
            <label>${t('auth.password')}</label>
            <input type="password" name="password" autocomplete="current-password" required />
          </div>
          <button type="submit" style="width:100%;justify-content:center;">${t('auth.login')}</button>
        </form>
      </div>
    </div>`;

  document.getElementById('login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const form = e.target;
    const btn = form.querySelector('button');
    btn.disabled = true;
    btn.textContent = t('auth.loggingIn');
    try {
      const res = await api.post('/api/auth/login', {
        username: form.username.value,
        password: form.password.value,
      });
      setToken(res.token);
      toast(t('auth.welcome', { name: res.user.username }));
      location.hash = '#/dashboard';
    } catch (err) {
      await alertDialog(t('auth.loginFailed'), err.message || t('auth.loginFailed'));
      btn.disabled = false;
      btn.textContent = t('auth.login');
    }
  });
}

export async function renderSetup(app) {
  app.innerHTML = `
    <div class="login-wrap">
      <div class="login-card">
        <h1>${t('auth.setupTitle')}</h1>
        <p class="subtitle">${t('auth.setupSubtitle')}</p>
        <form id="setup-form">
          <div class="field">
            <label>${t('auth.username')}</label>
            <input type="text" name="username" required placeholder="${t('auth.usernamePlaceholder')}" />
          </div>
          <div class="field">
            <label>${t('auth.passwordMin')}</label>
            <input type="password" name="password" required minlength="6" />
          </div>
          <div class="field">
            <label>${t('auth.confirmPassword')}</label>
            <input type="password" name="password2" required minlength="6" />
          </div>
          <button type="submit" style="width:100%;justify-content:center;">${t('auth.createAccount')}</button>
        </form>
      </div>
    </div>`;

  document.getElementById('setup-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const form = e.target;
    if (form.password.value !== form.password2.value) {
      await alertDialog(t('auth.passwordMismatch'), t('auth.passwordMismatch'));
      return;
    }
    const btn = form.querySelector('button');
    btn.disabled = true;
    btn.textContent = t('auth.creating');
    try {
      const res = await api.post('/api/users/bootstrap', {
        username: form.username.value,
        password: form.password.value,
      });
      invalidateSetup();
      toast(t('auth.setupDone', { name: res.username }));
      location.hash = '#/login';
    } catch (err) {
      await alertDialog(t('auth.setupFailed'), err.message || t('auth.setupFailed'));
      btn.disabled = false;
      btn.textContent = t('auth.createAccount');
    }
  });
}
