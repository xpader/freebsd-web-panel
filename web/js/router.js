// Hash-based router.

import { api } from './api.js';

const routes = [];
let currentRender = null;

export function defineRoute(path, handler) {
  routes.push({ path, handler });
}

// Whether an initial admin still needs to be created. Resolved lazily on the
// first unauthenticated visit and cached so we don't re-query on every
// navigation. Dropped after a successful bootstrap (see invalidateSetup).
let needsSetup = null;   // null = unknown, boolean = resolved
let setupReq = null;     // in-flight request, dedups concurrent callers

async function resolveNeedsSetup() {
  if (needsSetup !== null) return needsSetup;
  if (!setupReq) {
    setupReq = api
      .get('/api/users/bootstrap')
      .then((s) => { needsSetup = !!s.needs_setup; return needsSetup; })
      .catch(() => { needsSetup = false; return false; })
      .finally(() => { setupReq = null; });
  }
  return setupReq;
}

/// Drop the cache after the first admin is created so later unauthenticated
/// visits route to login instead of setup.
export function invalidateSetup() {
  needsSetup = false;
}

async function match() {
  let hash = location.hash.slice(1) || '/';
  if (hash === '/') hash = '/dashboard';

  // Find exact or prefix match (longest match wins).
  let best = null;
  for (const r of routes) {
    // A route ending with '/' is a "prefix-only" route (detail page parent).
    // It matches any sub-path but NOT the exact base path.
    const isPrefixRoute = r.path.endsWith('/') && r.path.length > 1;
    const rp = isPrefixRoute ? r.path.slice(0, -1) : r.path;
    const matched = isPrefixRoute
      ? hash.startsWith(rp + '/')     // /zfs/pools/ → only prefix, not exact
      : (hash === rp);                 // /zfs/pools → exact only
    if (matched) {
      if (!best || r.path.length > best.path.length) best = r;
    }
  }

  if (!best) {
    best = { handler: () => notFound() };
  }

  const token = sessionStorage.getItem('fwp_token');
  const isAuthRoute = hash === '/login' || hash === '/setup';

  // Logged-in users are never shown auth pages.
  if (token && isAuthRoute) {
    location.hash = '#/dashboard';
    return;
  }

  // Unauthenticated visitors are sent to first-run setup on a fresh install,
  // or to login otherwise — chosen automatically rather than by the user.
  if (!token) {
    const setup = await resolveNeedsSetup();
    const dest = setup ? '/setup' : '/login';
    if (hash !== dest) {
      location.hash = '#' + dest;
      return;
    }
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
