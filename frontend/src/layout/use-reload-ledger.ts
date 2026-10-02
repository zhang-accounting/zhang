import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { reloadLedger } from '@/api/requests';
import { apiErrorMessage } from '@/lib/api-error';

/** Asks the server to re-read the ledger files; the SSE `Reload` event (handled in App.tsx) refreshes the data. */
export function useReloadLedger() {
  const { t } = useTranslation();
  return useCallback(() => {
    toast.info(t('SHELL_RELOAD_SENT'), {
      id: 'leger-reload',
      description: t('SHELL_RELOAD_SENT_DESCRIPTION'),
    });
    reloadLedger({}).catch(async (error) => {
      toast.error(t('SHELL_RELOAD_FAILED'), { id: 'leger-reload', description: await apiErrorMessage(error) });
    });
  }, [t]);
}
