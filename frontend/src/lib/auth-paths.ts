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

/** Query parameters of the login page the app passes on to the code: its PKCE challenge (RFC 7636) and its `state`. */
export const APP_HANDOFF_PARAMS = ['code_challenge', 'code_challenge_method', 'state'] as const;

export type AppHandoffParams = Partial<Record<(typeof APP_HANDOFF_PARAMS)[number], string>>;

/**
 * The PKCE challenge and the state of the app (`/login?return_to=…&code_challenge=…&code_challenge_method=S256&state=…`), as
 * they are, for `POST /api/auth/app/code`; empty ones are left out. They stay in the URL of the login page through every way
 * of signing in. The server checks them (only the `S256` method) and refuses the code otherwise, which the handoff shows.
 */
export function appHandoffParams(search: string): AppHandoffParams {
  const query = new URLSearchParams(search);
  const params: AppHandoffParams = {};
  for (const name of APP_HANDOFF_PARAMS) {
    const value = query.get(name);
    if (value) params[name] = value;
  }
  return params;
}
