import { ApiError, Fetcher, type Middleware } from 'openapi-typescript-fetch';

import { paths } from './schemas';

// declare fetcher for paths
export const openAPIFetcher = Fetcher.for<paths>();

export const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';
export const backendUri: string = import.meta.env.VITE_API_ENDPOINT || 'http://localhost:8000';
/** Origin of the zhang server for links outside `/api` (OpenAPI docs): the backend itself in development. */
export const serverBaseUrl = development ? backendUri : '';
/**
 * Prefix of `/api` requests: always same-origin (the Vite dev server proxies `/api`, see vite.config.ts), so the HttpOnly
 * session cookie set by the login page goes with every call, uploads and the `/api/sse` EventSource.
 */
export const apiBaseUrl = '';

type UnauthorizedListener = () => void;
const unauthorizedListeners = new Set<UnauthorizedListener>();

/** Subscribes to "a ledger API call was refused with 401" (session expired or ended elsewhere). Returns the unsubscribe. */
export function onUnauthorized(listener: UnauthorizedListener) {
  unauthorizedListeners.add(listener);
  return () => {
    unauthorizedListeners.delete(listener);
  };
}

/**
 * Sign-in calls (`/api/auth/login`, `/api/auth/logout`, `/api/auth/passkey/*`) answer 401 for a wrong password / secret /
 * passkey: that is a form error, not a lost session. Every other `/api` endpoint (including `/api/auth/passkeys`) answers 401
 * only when there is no session.
 */
export function isSignInUrl(url: string) {
  try {
    const { pathname } = new URL(url, window.location.origin);
    return pathname === '/api/auth/login' || pathname === '/api/auth/logout' || pathname.startsWith('/api/auth/passkey/');
  } catch {
    return false;
  }
}

/** Call for every failed `/api` response: a 401 outside the sign-in calls sends the app back to the login page (`AuthGate`). */
export function reportUnauthorized(url: string, status: number) {
  if (status !== 401 || isSignInUrl(url)) return;
  unauthorizedListeners.forEach((listener) => listener());
}

/** The server answers `401 { message }` (no `WWW-Authenticate`, so no browser popup) once the session is gone. */
const sessionExpiry: Middleware = async (url, init, next) => {
  try {
    return await next(url, init);
  } catch (error) {
    if (error instanceof ApiError) reportUnauthorized(error.url || url, error.status);
    throw error;
  }
};

/**
 * zhang-server reads list query params as `tags[]=a&tags[]=b`: `tags=a` (what the generated client writes for arrays) is a
 * 400, and a percent-encoded `tags%5B%5D=a` is silently ignored. Rewrite the list params of `/api/journals`.
 */
const LIST_PARAMS = new Set(['tags', 'links']);
const listQueryParams: Middleware = (url, init, next) => {
  const index = url.indexOf('?');
  if (index === -1) return next(url, init);
  const query = url
    .slice(index + 1)
    .split('&')
    .map((pair) => {
      const eq = pair.indexOf('=');
      const key = eq === -1 ? pair : pair.slice(0, eq);
      return LIST_PARAMS.has(decodeURIComponent(key)) ? `${key}[]${eq === -1 ? '' : pair.slice(eq)}` : pair;
    })
    .join('&');
  return next(`${url.slice(0, index)}?${query}`, init);
};

// global configuration
openAPIFetcher.configure({
  baseUrl: apiBaseUrl,
  init: {},
  use: [sessionExpiry, listQueryParams], // middlewares
});
