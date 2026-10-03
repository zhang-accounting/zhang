/** Route of the login page (rendered by `AuthGate`, outside the app shell). */
export const LOGIN_PATH = '/login';

/** `/login?next=<path>`: the page to return to after signing in. */
export function loginUrl(location: { pathname: string; search: string; hash: string }) {
  const next = `${location.pathname}${location.search}${location.hash}`;
  return next === '/' || next.startsWith(LOGIN_PATH) ? LOGIN_PATH : `${LOGIN_PATH}?next=${encodeURIComponent(next)}`;
}

/** Same-origin app path from `?next=` (no `//host` / `/\host` open redirects, no loop back to /login); `/` otherwise. */
export function returnPath(search: string) {
  const next = new URLSearchParams(search).get('next');
  if (!next || !next.startsWith('/') || next.startsWith('//') || next.startsWith('/\\') || next.startsWith(LOGIN_PATH)) return '/';
  return next;
}
