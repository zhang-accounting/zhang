import { useSetAtom } from 'jotai';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { apiBaseUrl } from '@/api/fetcher';
import { accountFetcher } from '@/states/account';
import { loadAuthStatusAtom } from '@/states/auth';
import { basicInfoFetcher, onlineAtom, updatableVersionAtom } from '@/states/basic';
import { commoditiesFetcher } from '@/states/commodity';
import { errorsFetcher } from '@/states/errors';
import { journalFetcher } from '@/states/journals';
import { optionsFetcher } from '@/states/options';
import { reloadFailureDetail } from './reload-failure';

/**
 * Server-sent events (`/api/sse`): ledger reloads, failed reloads, connection state, new versions. Used by the app behind
 * `AuthGate`, so the stream opens after sign-in and closes on sign-out (unmount). A stream refused for good (401 once the
 * session is gone) makes the auth status reload, which shows the login page.
 */
export function useServerEvents() {
  const { i18n } = useTranslation();
  const setLedgerOnline = useSetAtom(onlineAtom);
  const setUpdatableVersion = useSetAtom(updatableVersionAtom);
  const loadAuthStatus = useSetAtom(loadAuthStatusAtom);

  const refreshErrors = useSetAtom(errorsFetcher);
  const refreshAccounts = useSetAtom(accountFetcher);
  const refreshBasicInfo = useSetAtom(basicInfoFetcher);
  const refreshCommodities = useSetAtom(commoditiesFetcher);
  const refreshJournal = useSetAtom(journalFetcher);
  const refreshOptions = useSetAtom(optionsFetcher);

  useEffect(() => {
    // `i18n.t` (not a captured `t`) so toasts follow later language switches.
    const events = new EventSource(`${apiBaseUrl}/api/sse`);
    // "Connected" arrives on every page load; only worth a toast when it ends an offline period.
    let wasOffline = false;
    events.onmessage = (event) => {
      const data = JSON.parse(event.data);
      switch (data?.type) {
        case 'Reload':
          toast.success(i18n.t('SHELL_RELOAD_DONE'), {
            id: 'leger-reload',
            description: i18n.t('SHELL_RELOAD_DONE_DESCRIPTION'),
          });

          refreshErrors();
          refreshAccounts();
          refreshBasicInfo();
          refreshCommodities();
          refreshJournal();
          refreshOptions();
          break;
        case 'ReloadFailed':
          // the server keeps serving the ledger loaded before; `/api/info` carries the failure for the notice in the shell
          toast.error(i18n.t('SHELL_RELOAD_FAILED'), {
            id: 'leger-reload',
            description: reloadFailureDetail(data),
          });
          refreshBasicInfo();
          break;
        case 'Connected':
          if (wasOffline) toast.success(i18n.t('SHELL_SERVER_CONNECTED'), { id: 'offline' });
          wasOffline = false;
          setLedgerOnline(true);
          refreshBasicInfo();
          break;
        case 'NewVersionFound':
          toast.info(i18n.t('SHELL_UPDATE_AVAILABLE', { version: data.version }));
          setUpdatableVersion(data.version);
          break;
        default:
          break;
      }
    };
    events.onerror = () => {
      wasOffline = true;
      setLedgerOnline(false);
      // CLOSED = the server answered with an error status (e.g. 401); EventSource does not retry those by itself.
      if (events.readyState === EventSource.CLOSED) void loadAuthStatus({ quiet: true });
      toast.error(i18n.t('SHELL_SERVER_OFFLINE'), {
        id: 'offline',
        description: i18n.t('SHELL_SERVER_OFFLINE_DESCRIPTION'),
      });
    };
    return () => {
      events.close();
      toast.dismiss('offline');
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
