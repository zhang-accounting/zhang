import { useSetAtom } from 'jotai';
import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { AuthRequestError, logout } from '@/api/auth';
import { apiErrorMessage } from '@/lib/api-error';
import { signedOutAtom } from '@/states/auth';

/**
 * Ends the session (`POST /api/auth/logout` clears the cookie) and shows the login page; `AuthGate` unmounts the shell, which
 * closes the SSE stream and drops the cached ledger data.
 */
export function useSignOut() {
  const { t } = useTranslation();
  const signedOut = useSetAtom(signedOutAtom);
  return useCallback(async () => {
    try {
      await logout();
    } catch (error) {
      // 401: the session was already gone, which is the goal.
      if (!(error instanceof AuthRequestError && error.status === 401)) {
        toast.error(t('auth.sign_out_failed'), { description: await apiErrorMessage(error) });
        return;
      }
    }
    signedOut('signed-out');
  }, [signedOut, t]);
}
