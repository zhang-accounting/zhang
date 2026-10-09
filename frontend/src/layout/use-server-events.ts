import { useSetAtom } from 'jotai';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { apiBaseUrl } from '@/api/fetcher';
import { loadAuthStatusAtom } from '@/states/auth';
import { basicInfoFetcher, onlineAtom, updatableVersionAtom } from '@/states/basic';
import { ledgerChangedAtom } from '@/states/ledger';
import { reloadFailureDetail } from './reload-failure';
import { serverEventHandler } from './server-events';

/**
 * Server-sent events (`/api/sse`): ledger reloads, failed reloads, connection state, new versions. Used by the app behind
 * `AuthGate`, so the stream opens after sign-in and closes on sign-out (unmount). A stream refused for good (401 once the
 * session is gone) makes the auth status reload, which shows the login page. What each event does is `serverEventHandler`.
 */
export function useServerEvents() {
  const { i18n } = useTranslation();
  const setLedgerOnline = useSetAtom(onlineAtom);
  const setUpdatableVersion = useSetAtom(updatableVersionAtom);
  const loadAuthStatus = useSetAtom(loadAuthStatusAtom);

  const ledgerChanged = useSetAtom(ledgerChangedAtom);
  const refreshBasicInfo = useSetAtom(basicInfoFetcher);

  useEffect(() => {
    // `i18n.t` (not a captured `t`) so toasts follow later language switches.
    const handler = serverEventHandler({
      ledgerChanged,
      reloaded: () => {
        toast.success(i18n.t('SHELL_RELOAD_DONE'), {
          id: 'leger-reload',
          description: i18n.t('SHELL_RELOAD_DONE_DESCRIPTION'),
        });
      },
      reloadFailed: (failure) => {
        // the server keeps serving the ledger loaded before; `/api/info` carries the failure for the notice in the shell
        toast.error(i18n.t('SHELL_RELOAD_FAILED'), {
          id: 'leger-reload',
          description: reloadFailureDetail(failure),
        });
        refreshBasicInfo();
      },
      connected: (afterOutage) => {
        // "Connected" arrives on every page load; only worth a toast when it ends an offline period.
        if (afterOutage) toast.success(i18n.t('SHELL_SERVER_CONNECTED'), { id: 'offline' });
        setLedgerOnline(true);
        refreshBasicInfo();
      },
      newVersion: (version) => {
        toast.info(i18n.t('SHELL_UPDATE_AVAILABLE', { version }));
        setUpdatableVersion(version);
      },
      disconnected: (closed) => {
        setLedgerOnline(false);
        if (closed) void loadAuthStatus({ quiet: true });
        toast.error(i18n.t('SHELL_SERVER_OFFLINE'), {
          id: 'offline',
          description: i18n.t('SHELL_SERVER_OFFLINE_DESCRIPTION'),
        });
      },
    });
    const events = new EventSource(`${apiBaseUrl}/api/sse`);
    events.onmessage = (event) => handler.message(JSON.parse(event.data));
    // CLOSED = the server answered with an error status (e.g. 401); EventSource does not retry those by itself.
    events.onerror = () => handler.error(events.readyState === EventSource.CLOSED);
    return () => {
      events.close();
      toast.dismiss('offline');
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
