import { useAtomValue, useSetAtom } from 'jotai';
import { RotateCw, ServerCrash } from 'lucide-react';
import { ReactNode, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Navigate, useLocation } from 'react-router';
import { onUnauthorized } from '@/api/fetcher';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { LOGIN_PATH, appReturnTo, loginUrl, returnPath } from '@/lib/auth-paths';
import Login from '@/pages/Login';
import { authLockedAtom, authStateAtom, loadAuthStatusAtom, resetLedgerStateAtom, signedOutAtom } from '@/states/auth';
import { AppHandoff } from './AppHandoff';
import { AuthScreen } from './AuthScreen';

/**
 * Renders the app only once `/api/auth/status` allows it. With auth on and no session the login page is shown at `/login`
 * (the requested path rides along in `?next=`); a 401 from any ledger API call (session expired) brings it back. The children
 * (shell, SSE connection, ledger atoms) are unmounted while signed out, so the SSE stream starts after sign-in and stops on
 * sign-out. Opened by the mobile app as `/login?return_to=<app url>`, the login page hands the session off to the app once
 * signed in (or right away when already signed in) instead of entering the web app, see `AppHandoff`.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const location = useLocation();
  const state = useAtomValue(authStateAtom);
  const locked = useAtomValue(authLockedAtom);
  const loadStatus = useSetAtom(loadAuthStatusAtom);
  const signedOut = useSetAtom(signedOutAtom);
  const resetLedgerState = useSetAtom(resetLedgerStateAtom);

  useEffect(() => {
    loadStatus();
  }, [loadStatus]);

  useEffect(() => onUnauthorized(() => signedOut('expired')), [signedOut]);

  // Passive effects of the unmounted shell (jotai unsubscriptions) run before this one, so the reset does not refetch.
  useEffect(() => {
    if (locked) resetLedgerState();
  }, [locked, resetLedgerState]);

  if (state.phase === 'loading') {
    return (
      <AuthScreen>
        <div className="flex flex-col items-center gap-4" aria-busy>
          <img src="/otter-192.png" alt="" className="size-12 rounded-lg" />
          <Spinner className="size-5 text-muted-foreground" aria-label={t('auth.loading')} />
        </div>
      </AuthScreen>
    );
  }

  if (state.phase === 'error') {
    return (
      <AuthScreen>
        <div className="flex w-full max-w-sm flex-col items-center gap-4 text-center">
          <ServerCrash className="size-10 text-muted-foreground" aria-hidden />
          <div className="flex flex-col gap-1">
            <h1 className="text-lg font-semibold">{t('auth.status_failed_title')}</h1>
            <p className="text-sm text-muted-foreground">{t('auth.status_failed_description')}</p>
            <p className="text-xs break-words text-muted-foreground">{state.message}</p>
          </div>
          <Button variant="outline" className="h-10" onClick={() => loadStatus()}>
            <RotateCw />
            {t('auth.retry')}
          </Button>
        </div>
      </AuthScreen>
    );
  }

  const onLoginRoute = location.pathname === LOGIN_PATH;
  if (locked) {
    if (!onLoginRoute) return <Navigate to={state.reason === 'signed-out' ? LOGIN_PATH : loginUrl(location)} replace />;
    return <Login status={state.status} reason={state.reason} />;
  }
  if (onLoginRoute) {
    const returnTo = state.status.enabled && state.status.app_login ? appReturnTo(location.search, state.status.app_return_schemes) : null;
    if (returnTo) return <AppHandoff returnTo={returnTo} />;
    return <Navigate to={returnPath(location.search)} replace />;
  }
  return children;
}
