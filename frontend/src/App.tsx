import { useLocalStorage } from '@/hooks/use-local-storage';

import { useTheme } from 'next-themes';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { basicInfoFetcher, onlineAtom, updatableVersionAtom } from './states/basic';
import { Router } from './router';
import { useSetAtom } from 'jotai';
import { errorsFetcher } from './states/errors';
import { accountFetcher } from './states/account';
import { commoditiesFetcher } from './states/commodity';
import { journalFetcher } from './states/journals';
import { toast } from 'sonner';
import { AppShell } from './layout/AppShell';

/** `--background` in light / dark (see global.css); used for `<meta name="theme-color">` (browser chrome, PWA title bar). */
const THEME_COLOR = { light: '#fbfdfc', dark: '#101211' };

/** BCP 47 tag for `<html lang>` (screen-reader pronunciation, CJK font selection, hyphenation). */
const htmlLang = (language: string | undefined) => (language?.startsWith('zh') ? 'zh-CN' : 'en');

export default function App() {
  const { i18n } = useTranslation();
  const { resolvedTheme } = useTheme();
  const [lang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });

  const setLedgerOnline = useSetAtom(onlineAtom);
  const setUpdatableVersion = useSetAtom(updatableVersionAtom);

  const refreshErrors = useSetAtom(errorsFetcher);
  const refreshAccounts = useSetAtom(accountFetcher);
  const refreshBasicInfo = useSetAtom(basicInfoFetcher);
  const refreshCommodities = useSetAtom(commoditiesFetcher);
  const refreshJournal = useSetAtom(journalFetcher);

  useEffect(() => {
    if (i18n.language !== lang) {
      i18n.changeLanguage(lang);
    }
  }, [i18n, lang]);

  // index.html ships one theme-color per OS scheme; once the app knows the chosen theme (which may differ from the OS), follow it.
  useEffect(() => {
    if (!resolvedTheme) return;
    const color = resolvedTheme === 'dark' ? THEME_COLOR.dark : THEME_COLOR.light;
    document.querySelectorAll('meta[name="theme-color"]').forEach((meta) => meta.setAttribute('content', color));
  }, [resolvedTheme]);

  useEffect(() => {
    const apply = (language: string) => {
      document.documentElement.lang = htmlLang(language);
    };
    apply(i18n.language);
    i18n.on('languageChanged', apply);
    return () => i18n.off('languageChanged', apply);
  }, [i18n]);

  useEffect(() => {
    // `i18n.t` (not a captured `t`) so toasts follow later language switches.
    const events = new EventSource('/api/sse');
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
      toast.error(i18n.t('SHELL_SERVER_OFFLINE'), {
        id: 'offline',
        description: i18n.t('SHELL_SERVER_OFFLINE_DESCRIPTION'),
      });
    };
    return () => events.close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <AppShell>
      <Router />
    </AppShell>
  );
}
