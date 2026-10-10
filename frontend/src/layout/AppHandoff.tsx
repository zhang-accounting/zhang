import { Smartphone } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router';
import { createAppCode } from '@/api/auth';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { apiErrorMessage } from '@/lib/api-error';
import { AuthScreen } from './AuthScreen';

/**
 * Mobile app login handoff: the app opened `/login?return_to=<app url>` in the system browser. Once signed in, this asks the
 * server for a one-time code of the session and opens the app URL carrying it (`?code=`); the app exchanges the code for the
 * session token. "Continue in the browser" stays for a browser without the app.
 */
export function AppHandoff({ returnTo }: { returnTo: string }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [error, setError] = useState<string>();
  const started = useRef(false);

  const handOff = useCallback(async () => {
    setError(undefined);
    try {
      const { redirect } = await createAppCode(returnTo);
      window.location.replace(redirect);
    } catch (failure) {
      setError(await apiErrorMessage(failure));
    }
  }, [returnTo]);

  useEffect(() => {
    // once per page: a second code would only expire unused
    if (started.current) return;
    started.current = true;
    void handOff();
  }, [handOff]);

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
          <Button variant="link" className="h-10" onClick={() => navigate('/', { replace: true })}>
            {t('auth.app_continue_in_browser')}
          </Button>
        </div>
      </div>
    </AuthScreen>
  );
}
