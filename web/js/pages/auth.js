// Login page + first-run setup.

import { api, setToken } from '../api.js';
import { invalidateSetup } from '../router.js';
import { toast } from '../ui/toast.js';

export async function renderLogin(app) {
  app.innerHTML = `
    <div class="login-wrap">
      <div class="login-card">
        <h1>FreeBSD Web Panel</h1>
        <p class="subtitle">请登录以管理系统</p>
        <form id="login-form">
          <div class="field">
            <label>用户名</label>
            <input type="text" name="username" autocomplete="username" required />
          </div>
          <div class="field">
            <label>密码</label>
            <input type="password" name="password" autocomplete="current-password" required />
          </div>
          <button type="submit" style="width:100%;justify-content:center;">登录</button>
        </form>
      </div>
    </div>`;

  document.getElementById('login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const form = e.target;
    const btn = form.querySelector('button');
    btn.disabled = true;
    btn.textContent = '登录中…';
    try {
      const res = await api.post('/api/auth/login', {
        username: form.username.value,
        password: form.password.value,
      });
      setToken(res.token);
      toast(`欢迎，${res.user.username}`);
      location.hash = '#/dashboard';
    } catch (err) {
      toast(err.message || '登录失败', 'error');
      btn.disabled = false;
      btn.textContent = '登录';
    }
  });
}

export async function renderSetup(app) {
  app.innerHTML = `
    <div class="login-wrap">
      <div class="login-card">
        <h1>初始化管理员</h1>
        <p class="subtitle">创建首个管理员账户以开始使用</p>
        <form id="setup-form">
          <div class="field">
            <label>用户名</label>
            <input type="text" name="username" required placeholder="2-32 位字母数字 _ . -" />
          </div>
          <div class="field">
            <label>密码（至少 6 位）</label>
            <input type="password" name="password" required minlength="6" />
          </div>
          <div class="field">
            <label>确认密码</label>
            <input type="password" name="password2" required minlength="6" />
          </div>
          <button type="submit" style="width:100%;justify-content:center;">创建账户</button>
        </form>
      </div>
    </div>`;

  document.getElementById('setup-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const form = e.target;
    if (form.password.value !== form.password2.value) {
      toast('两次密码不一致', 'error');
      return;
    }
    const btn = form.querySelector('button');
    btn.disabled = true;
    btn.textContent = '创建中…';
    try {
      const res = await api.post('/api/users/bootstrap', {
        username: form.username.value,
        password: form.password.value,
      });
      invalidateSetup();
      toast(`管理员 ${res.username} 创建成功，请登录`);
      location.hash = '#/login';
    } catch (err) {
      toast(err.message || '创建失败', 'error');
      btn.disabled = false;
      btn.textContent = '创建账户';
    }
  });
}
