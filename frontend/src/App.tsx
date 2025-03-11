import { useLocalStorage } from '@mantine/hooks';

import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { basicInfoFetcher, onlineAtom, updatableVersionAtom } from './states/basic';
import { Router } from './router';
import { useSetAtom } from 'jotai';
import { errorsFetcher } from './states/errors';
import { accountFetcher } from './states/account';
import { commoditiesFetcher } from './states/commodity';
import { journalFetcher } from './states/journals';
import Sidebar from './layout/Sidebar.tsx';
import { Nav } from './layout/Nav.tsx';
import { toast } from 'sonner';
import NetworkStatus from './components/NetworkStatus';
import MobileNavBar from './components/MobileNavBar';

export default function App() {
  const { i18n } = useTranslation();
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

  useEffect(() => {
    let events = new EventSource('/api/sse');
    events.onmessage = (event) => {
      console.log(event);
      const data = JSON.parse(event.data);
      switch (data?.type) {
        case 'Reload':
          toast.success('[Ledger Reload] reloaded', {
            id: 'leger-reload',
            description: 'reloading latest ledger info',
          });

          refreshErrors();
          refreshAccounts();
          refreshBasicInfo();
          refreshCommodities();
          refreshJournal();
          break;
        case 'Connected':
          toast.success('Connected to server');
          setLedgerOnline(true);
          refreshBasicInfo();
          break;
        case 'NewVersionFound':
          toast.info('New Version Found');
          setUpdatableVersion(data.version);
          break;
        default:
          break;
      }
    };
    events.onerror = () => {
      setLedgerOnline(false);
      toast.error('Server Offline', {
        id: 'offline',
        description: 'Client can not connect to server',
      });
    };
  }, []);

  return (
    <>
      <div className="grid h-screen w-full md:grid-cols-[220px_1fr] lg:grid-cols-[220px_1fr]">
        <Sidebar />
        <div className="flex flex-col sm:gap-4 sm:py-4 overflow-hidden">
          <Nav />
          <main className="grid flex-1 items-start gap-4 p-4 sm:px-6 sm:py-0 md:gap-8 overflow-scroll pb-16 sm:pb-0">
            <Router />
          </main>
        </div>
      </div>
      <MobileNavBar />
      <NetworkStatus />
    </>
  );
}
