import { Smartphone } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router';
import { createAppCode } from '@/api/auth';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { apiErrorMessage } from '@/lib/api-error';
import { type AppHandoffParams, appTarget } from '@/lib/auth-paths';
import { AuthScreen } from './AuthScreen';

/**
 * Mobile app login handoff: the app opened `/login?return_to=<app url>` in the system browser, with its PKCE challenge and state
 * (`params`). Once signed in, this asks the server for a one-time code of the session, bound to the challenge, and opens the app
 * URL carrying it (`?code=…&state=…`); the app exchanges the code and its code verifier for the session token. A challenge the
 * server refuses (any method but `S256`) gets no code and shows the error. "Continue in the browser" stays for a browser without
 * the app.
 *
 * `needsConsent` (the browser was signed in before this page, see `handoffNeedsConsent`): nothing is created until the user
 * answers "Sign in to the app as <user>?", which names the scheme and host of the app URL. Right after a sign-in on this page the
 * code is created at once.
 */
export function AppHandoff({
  returnTo,
  params,
  user,
  needsConsent,
}: {
  returnTo: string;
  params: AppHandoffParams;
  user?: string | null;
  needsConsent: boolean;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [step, setStep] = useState<'consent' | 'handoff' | 'cancelled'>(needsConsent ? 'consent' : 'handoff');
  const [error, setError] = useState<string>();
  const started = useRef(false);
  const { code_challenge, code_challenge_method, state } = params;

  const handOff = useCallback(async () => {
    setError(undefined);
    try {
      const { redirect } = await createAppCode(returnTo, { code_challenge, code_challenge_method, state });
      window.location.replace(redirect);
    } catch (failure) {
      setError(await apiErrorMessage(failure));
    }
  }, [returnTo, code_challenge, code_challenge_method, state]);

  useEffect(() => {
    // once per page, and only once the user agreed: a second code would only expire unused
    if (step !== 'handoff' || started.current) return;
    started.current = true;
    void handOff();
  }, [step, handOff]);

  const continueInBrowser = (
    <Button variant="link" className="h-10" onClick={() => navigate('/', { replace: true })}>
      {t('auth.app_continue_in_browser')}
    </Button>
  );

  if (step === 'consent') {
    return (
      <AuthScreen>
        <div className="flex w-full max-w-sm flex-col items-center gap-4 text-center">
          <img src="/otter-192.png" alt="" className="size-12 rounded-lg" />
          <div className="flex flex-col gap-2">
            <h1 className="text-lg font-semibold text-balance">{user ? t('auth.app_consent_title', { user }) : t('auth.app_consent_title_no_user')}</h1>
            <p className="text-sm text-muted-foreground">{t('auth.app_consent_target')}</p>
            <p className="rounded-md bg-muted px-3 py-2 font-mono text-sm break-all">{appTarget(returnTo)}</p>
            <p className="text-sm text-balance text-muted-foreground">{t('auth.app_consent_warning')}</p>
          </div>
          <div className="flex w-full flex-col gap-2">
            <Button className="h-10" onClick={() => setStep('handoff')}>
              <Smartphone />
              {t('auth.app_consent_continue')}
            </Button>
            <Button variant="outline" className="h-10" onClick={() => setStep('cancelled')}>
              {t('auth.app_consent_cancel')}
            </Button>
          </div>
        </div>
      </AuthScreen>
    );
  }

  if (step === 'cancelled') {
    return (
      <AuthScreen>
        <div className="flex w-full max-w-sm flex-col items-center gap-4 text-center">
          <img src="/otter-192.png" alt="" className="size-12 rounded-lg" />
          <p role="status" className="text-sm text-balance text-muted-foreground">
            {t('auth.app_consent_cancelled')}
          </p>
          {continueInBrowser}
        </div>
      </AuthScreen>
    );
  }

  return (
    <AuthScreen>
      <div className="flex w-full max-w-sm flex-col items-center gap-4 text-center" aria-busy={!error}>
        <img src="/otter-192.png" alt="" className="size-12 rounded-lg" />
        {error ? (
          <p role="alert" className="text-sm text-balance text-destructive">
            {t('auth.app_return_failed', { message: error })}
          </p>
        ) : (
          <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner role="presentation" aria-hidden className="size-4" />
            {t('auth.app_returning')}
          </p>
        )}
        <div className="flex flex-col items-center gap-2">
          {error && (
            <Button variant="outline" className="h-10" onClick={() => void handOff()}>
              <Smartphone />
              {t('auth.retry')}
            </Button>
          )}
          {continueInBrowser}
        </div>
      </div>
    </AuthScreen>
  );
}
