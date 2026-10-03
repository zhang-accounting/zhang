import { atom } from 'jotai';
import { AUTH_DISABLED, AuthRequestError, fetchAuthStatus, type AuthStatus } from '@/api/auth';
import { apiErrorMessage } from '@/lib/api-error';
import { accountFetcher } from './account';
import { basicInfoFetcher, onlineAtom } from './basic';
import { commoditiesFetcher } from './commodity';
import { errorsFetcher } from './errors';
import { journalFetcher } from './journals';

/** Why the login page is showing: `expired` adds a "session expired" note, `signed-out` drops the return path. */
export type LockReason = 'expired' | 'signed-out';

export type AuthState = { phase: 'loading' } | { phase: 'error'; message: string } | { phase: 'ready'; status: AuthStatus; reason?: LockReason };

/** Auth is on and there is no session: the login page replaces the app. */
const isLocked = (status: AuthStatus) => status.enabled && !status.authenticated;

/** `GET /api/auth/status`, loaded once by `AuthGate` and kept in sync on sign-in / sign-out / 401. */
export const authStateAtom = atom<AuthState>({ phase: 'loading' });

export const authStatusAtom = atom((get) => {
  const state = get(authStateAtom);
  return state.phase === 'ready' ? state.status : undefined;
});

/** The login page is shown instead of the app. */
export const authLockedAtom = atom((get) => {
  const status = get(authStatusAtom);
  return !!status && isLocked(status);
});

/** Signed in to an instance with auth on ("Sign out" in the sidebar / More sheet). */
export const canSignOutAtom = atom((get) => {
  const status = get(authStatusAtom);
  return !!status && status.enabled && status.authenticated;
});

/** `ZHANG_PASSKEY` is set (Settings → Passkeys). */
export const passkeyModeAtom = atom((get) => !!get(authStatusAtom)?.methods.passkey);

/**
 * (Re)loads the status. `quiet` keeps the current screen while it loads and ignores failures (background refresh after
 * sign-in / sign-out / passkey changes / a closed SSE stream). A quiet refresh never leaves the login page (only a sign-in
 * does): a ledger call answered 401 even if the status still claims a session, and flipping back would loop. A server without
 * the auth endpoints (404) behaves as "auth disabled".
 */
export const loadAuthStatusAtom = atom(null, async (get, set, options?: { quiet?: boolean }) => {
  const quiet = options?.quiet ?? false;
  if (!quiet) set(authStateAtom, { phase: 'loading' });
  let status: AuthStatus;
  try {
    status = await fetchAuthStatus();
  } catch (error) {
    if (error instanceof AuthRequestError && error.status === 404) status = AUTH_DISABLED;
    else {
      if (!quiet) set(authStateAtom, { phase: 'error', message: await apiErrorMessage(error) });
      return;
    }
  }
  const current = get(authStateAtom);
  if (quiet && current.phase === 'ready' && isLocked(current.status) && !isLocked(status)) {
    status = { ...status, enabled: true, authenticated: false };
  }
  if (current.phase === 'ready' && JSON.stringify(current.status) === JSON.stringify(status)) return;
  set(authStateAtom, { phase: 'ready', status, reason: current.phase === 'ready' ? current.reason : undefined });
});

/** Sign-in succeeded (password or passkey). */
export const signedInAtom = atom(null, (get, set) => {
  const state = get(authStateAtom);
  if (state.phase !== 'ready') return;
  set(authStateAtom, { phase: 'ready', status: { ...state.status, authenticated: true } });
  void set(loadAuthStatusAtom, { quiet: true });
});

/** Back to the login page: after "Sign out", or when an API call answered 401 (`expired`). */
export const signedOutAtom = atom(null, (get, set, reason: LockReason) => {
  const state = get(authStateAtom);
  if (state.phase !== 'ready') return;
  if (reason === 'expired' && state.status.enabled && !state.status.authenticated) return; // already on the login page
  set(authStateAtom, { phase: 'ready', status: { ...state.status, enabled: true, authenticated: false }, reason });
  void set(loadAuthStatusAtom, { quiet: true });
});

/**
 * Drops the cached ledger data (and the SSE online flag) so nothing from the previous session is shown after the next
 * sign-in. Run once the app shell has unmounted: refreshing a mounted `atomWithRefresh` would refetch (and 401) right away.
 */
export const resetLedgerStateAtom = atom(null, (_get, set) => {
  set(basicInfoFetcher);
  set(accountFetcher);
  set(commoditiesFetcher);
  set(errorsFetcher);
  set(journalFetcher);
  set(onlineAtom, false);
});
