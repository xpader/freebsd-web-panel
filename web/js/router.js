// Hash-based router.

const routes = [];
let currentRender = null;

export function defineRoute(path, handler) {
  routes.push({ path, handler });
}

function match() {
  let hash = location.hash.slice(1) || '/';
  if (hash === '/') hash = '/dashboard';

  // Find exact or prefix match.
  let best = null;
  for (const r of routes) {
    if (hash === r.path || hash.startsWith(r.path + '/')) {
      if (!best || r.path.length > best.path.length) best = r;
    }
  }

  if (!best) {
    best = { handler: () => notFound() };
  }

  // Check auth: if not logged in and not on login/bootstrap, redirect.
  const token = sessionStorage.getItem('fwp_token');
  const isAuthRoute = hash === '/login' || hash === '/setup';
  if (!token && !isAuthRoute) {
    // Need to check bootstrap status first — handled in main.
    location.hash = '#/login';
    return;
  }
  if (token && isAuthRoute) {
    location.hash = '#/dashboard';
    return;
  }

  const app = document.getElementById('app');
  best.handler(app, hash);
}

function notFound() {
  document.getElementById('app').innerHTML = `
    <div class="main"><div class="empty">
      <h1>404</h1><p>页面不存在</p>
    </div></div>`;
}

export function startRouter() {
  window.addEventListener('hashchange', match);
  match();
}
