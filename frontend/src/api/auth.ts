import { apiBaseUrl } from './fetcher';
import { responseError } from '@/lib/api-error';
import type { CreationOptionsJSON, RequestOptionsJSON } from '@/lib/webauthn';

/** `GET /api/auth/status`. `enabled` = the server was started with `ZHANG_AUTH` and / or `ZHANG_PASSKEY`. */
export interface AuthStatus {
  enabled: boolean;
  authenticated: boolean;
  methods: { password: boolean; passkey: boolean };
  /** At least one passkey exists (otherwise the login page offers the first registration). */
  passkey_registered: boolean;
  user?: string | null;
  /** Ledger title, readable before signing in (`/api/info` is protected). */
  title?: string | null;
  /** The server supports the mobile app login handoff (`/login?return_to=<app url>`); absent on older servers. */
  app_login?: boolean;
  /** URL schemes the handoff may return to (`zhang-app` and `ZHANG_APP_RETURN_SCHEMES`). */
  app_return_schemes?: string[];
}

/** One-time code of the app login handoff and the app URL carrying it. */
export interface AppCode {
  code: string;
  redirect: string;
}

export interface PasskeyInfo {
  id: string;
  name: string;
  /** RFC 3339 string or a unix timestamp (seconds or milliseconds). */
  created_at: string | number;
}

/** First half of a WebAuthn ceremony: the server keeps the challenge under `state_id` until `finish`. */
export interface PasskeyChallenge<Options> {
  state_id: string;
  options: Options;
}

/** Failed auth request. `status` is the HTTP status, 0 when the server could not be reached. */
export class AuthRequestError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = 'AuthRequestError';
    this.status = status;
  }
}

/** Status of a server without the auth endpoints (404): behaves like "no auth", as before the login page existed. */
export const AUTH_DISABLED: AuthStatus = {
  enabled: false,
  authenticated: true,
  methods: { password: false, passkey: false },
  passkey_registered: false,
};

async function authRequest<T>(path: string, method: 'GET' | 'POST' | 'DELETE' = 'GET', body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${apiBaseUrl}/api/auth${path}`, {
      method,
      credentials: 'same-origin',
      headers: body === undefined ? { Accept: 'application/json' } : { Accept: 'application/json', 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (error) {
    throw new AuthRequestError(error instanceof Error ? error.message : String(error), 0);
  }
  if (!response.ok) throw new AuthRequestError((await responseError(response)).message, response.status);
  const text = await response.text();
  if (!text) return undefined as T;
  const json = JSON.parse(text);
  // zhang-server wraps payloads in `{ data }`; accept a bare payload too.
  return (json !== null && typeof json === 'object' && !Array.isArray(json) && 'data' in json ? json.data : json) as T;
}

export const fetchAuthStatus = () => authRequest<AuthStatus>('/status');

export const login = (username: string, password: string) => authRequest<void>('/login', 'POST', { username, password });

export const logout = () => authRequest<void>('/logout', 'POST');

/** Asks for a one-time code for the signed-in session; the app exchanges it for the session token. */
export const createAppCode = (returnTo: string) => authRequest<AppCode>('/app/code', 'POST', { return_to: returnTo });

/** `secret` is the `ZHANG_PASSKEY` value; `null` once signed in (adding another passkey from Settings). */
export const startPasskeyRegistration = (secret: string | null, name: string | null) =>
  authRequest<PasskeyChallenge<CreationOptionsJSON>>('/passkey/register/start', 'POST', { secret, name });

/** Stores the new passkey and signs the browser in. */
export const finishPasskeyRegistration = (stateId: string, name: string | null, credential: unknown) =>
  authRequest<void>('/passkey/register/finish', 'POST', { state_id: stateId, name, credential });

export const startPasskeyLogin = () => authRequest<PasskeyChallenge<RequestOptionsJSON>>('/passkey/login/start', 'POST');

export const finishPasskeyLogin = (stateId: string, credential: unknown) =>
  authRequest<void>('/passkey/login/finish', 'POST', { state_id: stateId, credential });

export const listPasskeys = () => authRequest<PasskeyInfo[]>('/passkeys');

/** 409 `{ message }` when it is the last passkey and password sign-in is off (the ledger would be locked). */
export const deletePasskey = (id: string) => authRequest<void>(`/passkeys/${encodeURIComponent(id)}`, 'DELETE');
