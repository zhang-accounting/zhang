import { Link as RouteLink } from 'react-router-dom';
import { matchPath, useLocation } from 'react-router';
import { useDisclosure, useLocalStorage, useMediaQuery } from '@mantine/hooks';

import { ActionIcon, Anchor, AppShell, Badge, Box, Group, Stack, Text, TextInput, UnstyledButton } from '@mantine/core';
import {
  IconBroadcast,
  IconCash,
  IconChartAreaLine,
  IconCheck,
  IconCreditCard,
  IconCurrencyBitcoin,
  IconFiles,
  IconList,
  IconNotebook,
  IconReceipt2,
  IconRefresh,
  IconSearch,
  IconSettings,
  IconSmartHome,
  IconTools,
} from '@tabler/icons-react';
import NewTransactionButton from './components/NewTransactionButton';
import { notifications } from '@mantine/notifications';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { basicInfoFetcher, onlineAtom, titleAtom, updatableVersionAtom } from './states/basic';
import { useSWRConfig } from 'swr';
import { createStyles } from '@mantine/emotion';
import { Router } from './router';
import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import { errorCountAtom, errorsFetcher } from './states/errors';
import { accountFetcher } from './states/account';
import { commoditiesFetcher } from './states/commodity';
import { journalFetcher } from './states/journals';
import { axiosInstance, serverBaseUrl } from './global.ts';
import Sidebar from './layout/Sidebar.tsx';
import { Nav } from './layout/Nav.tsx';

const useStyles = createStyles((theme, _, u) => ({
  onlineIcon: {
    color: theme.colors.blue[6],
  },
  offlineIcon: {
    color: theme.colors.red[4],
  },

  header: {
    [u.dark]: {
      color: theme.white,
      borderBottom: `1px solid ${theme.colors.dark[4]}`,
    },
    [u.light]: {
      color: theme.black,
      borderBottom: `1px solid ${theme.colors.gray[3]}`,
    },
    padding: theme.spacing.sm,
    marginLeft: -theme.spacing.md,
    marginRight: -theme.spacing.md,
  },
  navbar: {
    paddingTop: 0,
  },

  section: {
    marginLeft: -theme.spacing.md,
    marginRight: -theme.spacing.md,
    marginBottom: theme.spacing.md,

    '&:not(:last-of-type)': {
      [u.dark]: {
        borderBottom: `1px solid ${theme.colors.dark[4]}`,
      },
      [u.light]: {
        borderBottom: `1px solid ${theme.colors.gray[3]}`,
      },
    },
  },

  searchCode: {
    fontWeight: 700,
    fontSize: 10,
    [u.dark]: {
      backgroundColor: theme.colors.dark[7],
      border: `1px solid ${theme.colors.dark[7]}`,
    },
    [u.light]: {
      backgroundColor: theme.colors.gray[0],
      border: `1px solid ${theme.colors.gray[2]}`,
    },
  },

  mainLinks: {
    paddingBottom: theme.spacing.md,
  },
  mainLink: {
    [u.dark]: { color: theme.colors.dark[0] },
    [u.light]: { color: theme.colors.gray[7] },
    display: 'flex',
    alignItems: 'center',
    width: '100%',
    fontSize: theme.fontSizes.sm,
    margin: `calc(${theme.spacing.sm} * 0.25) 0`,
    padding: `calc(${theme.spacing.sm} * 0.75) ${theme.spacing.xs}`,
    borderRadius: theme.radius.sm,
    fontWeight: 500,
    border: `2px solid transparent`,
    '&:hover': {
      [u.dark]: {
        color: theme.white,
        backgroundColor: theme.colors.dark[6],
      },
      [u.light]: {
        color: theme.black,
        backgroundColor: theme.colors.gray[0],
      },
      borderColor: theme.colors[theme.primaryColor][6],
    },
  },

  activeMainLink: {
    borderColor: theme.colors[theme.primaryColor][6],
  },

  mainLinkInner: {
    display: 'flex',
    alignItems: 'center',
    flex: 1,
  },

  mainLinkIcon: {
    [u.dark]: { color: theme.colors.dark[2] },
    [u.light]: { color: theme.colors.gray[6] },
    marginRight: theme.spacing.sm,
  },

  mainLinkBadge: {
    padding: 0,
    width: 20,
    height: 20,
    pointerEvents: 'none',
  },

  collections: {
    paddingLeft: theme.spacing.md,
    paddingRight: theme.spacing.md,
    paddingBottom: theme.spacing.md,
  },

  collectionsHeader: {
    paddingLeft: theme.spacing.md + 2,
    paddingRight: theme.spacing.md,
    marginBottom: 5,
  },

  collectionLink: {
    [u.dark]: { color: theme.colors.dark[0] },
    [u.light]: { color: theme.colors.gray[7] },

    display: 'block',
    padding: `8px ${theme.spacing.xs}`,
    textDecoration: 'none',
    borderRadius: theme.radius.sm,
    fontSize: theme.fontSizes.xs,
    lineHeight: 1,
    fontWeight: 500,

    '&:hover': {
      [u.dark]: {
        backgroundColor: theme.colors.dark[6],
        color: theme.white,
      },
      [u.light]: {
        backgroundColor: theme.colors.gray[0],
        color: theme.black,
      },
    },
  },
}));

interface LinkItem {
  icon: any;
  label: string;
  uri: string;
  notifications?: number;
}

const links: LinkItem[] = [
  { icon: IconList, label: 'NAV_JOURNALS', uri: '/journals' },
  { icon: IconCash, label: 'NAV_ACCOUNTS', uri: '/accounts' },
  { icon: IconCurrencyBitcoin, label: 'NAV_COMMODITIES', uri: '/commodities' },
  { icon: IconCurrencyBitcoin, label: 'NAV_BUDGETS', uri: '/budgets' },
  { icon: IconFiles, label: 'NAV_DOCUMENTS', uri: '/documents' },
  { icon: IconChartAreaLine, label: 'NAV_REPORT', uri: '/report' },
  { icon: IconCreditCard, label: 'NAV_LIABILITY', uri: '/liability' },
  { icon: IconNotebook, label: 'NAV_RAW_EDITING', uri: '/edit' },
  { icon: IconTools, label: 'NAV_TOOLS', uri: '/tools' },
  { icon: IconSettings, label: 'NAV_SETTING', uri: '/settings' },
];

export default function App() {
  const { mutate } = useSWRConfig();
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
    let events = new EventSource(serverBaseUrl + '/api/sse');
    events.onmessage = (event) => {
      console.log(event);
      const data = JSON.parse(event.data);
      switch (data?.type) {
        case 'Reload':
          notifications.update({
            id: 'leger-reload',
            title: '[Ledger Reload] reloaded',
            message: 'reloading latest ledger info',
            icon: <IconCheck />,
            color: 'teal',
            loading: false,
            autoClose: 3000,
          });
          mutate('/api/for-new-transaction');
          refreshErrors();
          refreshAccounts();
          refreshBasicInfo();
          refreshCommodities();
          refreshJournal();
          break;
        case 'Connected':
          notifications.show({
            title: 'Connected to server',
            icon: <IconBroadcast />,
            message: '',
          });
          setLedgerOnline(true);
          refreshBasicInfo();
          break;
        case 'NewVersionFound':
          setUpdatableVersion(data.version);
          break;
        default:
          break;
      }
    };
    events.onerror = () => {
      setLedgerOnline(false);
      notifications.show({
        id: 'offline',
        title: 'Server Offline',
        icon: <IconBroadcast />,
        color: 'red',
        message: 'Client can not connect to server',
      });
    };
  }, [mutate]); // eslint-disable-line


  return (
    <div className="grid min-h-screen w-full md:grid-cols-[180px_1fr] lg:grid-cols-[280px_1fr]">
      <Sidebar />
      <div className="flex flex-col sm:gap-4 sm:py-4">
        <Nav />
        <main className="grid flex-1 items-start gap-4 p-4 sm:px-6 sm:py-0 md:gap-8">
          <Router />
        </main>
      </div>
    </div>
  );

}
