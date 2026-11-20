/** Route of the login page (rendered by `AuthGate`, outside the app shell). */
export const LOGIN_PATH = '/login';

/** `/login?next=<path>`: the page to return to after signing in. */
export function loginUrl(location: { pathname: string; search: string; hash: string }) {
  const next = `${location.pathname}${location.search}${location.hash}`;
  return next === '/' || next.startsWith(LOGIN_PATH) ? LOGIN_PATH : `${LOGIN_PATH}?next=${encodeURIComponent(next)}`;
}

/** Any origin will do: `returnPath` only reads the path the browser would make of `?next=` on its own origin. */
const SAME_ORIGIN = 'http://zhang.invalid';

/**
 * Same-origin app path from `?next=`, as the browser will normalise it (`/./login`, `/a/../login` and `/%2e/login` are
 * `/login`); `/` for `//host` and anything with a backslash (open redirects), and for the login page itself: a sign-in must
 * never land on `/login?return_to=…` and hand its session to an app without asking.
 */
export function returnPath(search: string) {
  const next = new URLSearchParams(search).get('next');
  if (!next || !next.startsWith('/') || next.startsWith('//') || next.includes('\\')) return '/';
  let url: URL;
  try {
    url = new URL(next, SAME_ORIGIN);
  } catch {
    return '/';
  }
  if (url.origin !== SAME_ORIGIN || url.pathname.startsWith(LOGIN_PATH)) return '/';
  return `${url.pathname}${url.search}${url.hash}`;
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

/** Scheme and host of the app URL (`zhang-app://auth`), shown when the handoff asks for consent; the scheme alone without a host. */
export function appTarget(returnTo: string) {
  try {
    const url = new URL(returnTo);
    return url.host ? `${url.protocol}//${url.host}` : url.protocol;
  } catch {
    return returnTo;
  }
}

/**
 * Whether the handoff asks "Sign in to the app as <user>?" before it creates the code for `returnTo`. `signedInFor` is the app
 * URL the login page showed when the user signed in on it (`null` without one, and once its code was created): right after that
 * sign-in the user just acted, so the code is created at once. Any other session, the one the browser already had above all,
 * needs the click. Otherwise any app that registered the scheme could open the login page in a browser sharing that session and
 * get a session token without the user noticing (client impersonation, RFC 8252 section 8.6), which PKCE does not prevent: that
 * app brings its own verifier.
 */
export function handoffNeedsConsent(signedInFor: string | null, returnTo: string) {
  return signedInFor !== returnTo;
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
