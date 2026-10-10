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

/** Query parameter of the login page with the app URL to return to (mobile app login handoff). */
export const APP_RETURN_PARAM = 'return_to';

/**
 * The app URL of `?return_to=` when its scheme is one the server allows (`schemes` of `/api/auth/status`), else `null`:
 * the login page then behaves as a normal web login. The server checks the scheme again when it issues the code.
 */
export function appReturnTo(search: string, schemes: readonly string[] | undefined) {
  const returnTo = new URLSearchParams(search).get(APP_RETURN_PARAM)?.trim();
  if (!returnTo || !schemes?.length) return null;
  let scheme: string;
  try {
    scheme = new URL(returnTo).protocol.replace(/:$/, '').toLowerCase();
  } catch {
    return null;
  }
  return schemes.some((allowed) => allowed.toLowerCase() === scheme) ? returnTo : null;
}
