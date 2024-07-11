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
import { serverBaseUrl } from './index';
import { useAppDispatch, useAppSelector } from './states';
import { accountsSlice, fetchAccounts } from './states/account';
import { basicInfoSlice, fetchBasicInfo, reloadLedger } from './states/basic';
import { fetchCommodities } from './states/commodity';
import { journalsSlice } from './states/journals';
import { useSWRConfig } from 'swr';
import { createStyles } from '@mantine/emotion';
import { Router } from './router';
import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import { errorAtom, errorCountAtom, errorsFetcher } from './states/errors';

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
  const { classes } = useStyles();
  const { t, i18n } = useTranslation();
  const dispatch = useAppDispatch();
  const basicInfo = useAppSelector((state) => state.basic);
  const location = useLocation();
  const [lang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });
  const [opened] = useDisclosure();
  const isMobile = useMediaQuery('(max-width: 768px)');

  const errorsCount = useAtomValue(errorCountAtom);
  const refreshErrors = useSetAtom(errorsFetcher);

  useEffect(() => {
    if (i18n.language !== lang) {
      i18n.changeLanguage(lang);
    }
  }, [i18n, lang]);

  useEffect(() => {
    dispatch(fetchCommodities());
    dispatch(fetchBasicInfo());
    dispatch(fetchAccounts());

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
          dispatch(fetchBasicInfo());
          dispatch(fetchCommodities());
          dispatch(accountsSlice.actions.clear());
          dispatch(journalsSlice.actions.clear());
          break;
        case 'Connected':
          notifications.show({
            title: 'Connected to server',
            icon: <IconBroadcast />,
            message: '',
          });
          dispatch(fetchBasicInfo());
          break;
        case 'NewVersionFound':
          dispatch(basicInfoSlice.actions.setUpdatableVersion({ newVersion: data.version }));
          break;
        default:
          break;
      }
    };
    events.onerror = () => {
      dispatch(basicInfoSlice.actions.offline());
      notifications.show({
        id: 'offline',
        title: 'Server Offline',
        icon: <IconBroadcast />,
        color: 'red',
        message: 'Client can not connect to server',
      });
    };
  }, [dispatch, mutate]);

  const sendReloadEvent = () => {
    notifications.show({
      id: 'leger-reload',
      title: '[Ledger Reload] reload event is sent',
      message: 'please wait for ledger reload',
      loading: true,
      autoClose: false,
    });
    dispatch(reloadLedger());
  };

  const mainLinks = links.map((link) => (
    <UnstyledButton
      component={RouteLink}
      to={link.uri}
      key={link.label}
      className={`${classes.mainLink} ${matchPath(location.pathname, link.uri) ? classes.activeMainLink : ''}`}
    >
      <div className={classes.mainLinkInner}>
        <link.icon size={20} className={classes.mainLinkIcon} stroke={1.5} />
        <span>{t(link.label)}</span>
      </div>
      {link.notifications && (
        <Badge size="sm" variant="filled" className={classes.mainLinkBadge}>
          {link.notifications}
        </Badge>
      )}
    </UnstyledButton>
  ));

  const mobileMainLinks = links.map((link) => (
    <UnstyledButton component={RouteLink} to={link.uri} key={link.label}>
      <link.icon size={20} stroke={1.5} />
    </UnstyledButton>
  ));
  return (
    <AppShell padding="xs" header={{ height: 128, collapsed: !isMobile }} navbar={{ width: 240, breakpoint: 'sm', collapsed: { mobile: !opened } }}>
      {isMobile && (
        <AppShell.Header>
          <Box m="xs">
            <Group justify="space-between">
              <Group gap="xs" justify="left">
                <IconReceipt2 stroke={1.5} />
                <Text>ZHANG</Text>
              </Group>
              <NewTransactionButton />
            </Group>
            <Group justify="space-between" mt="xs">
              {mobileMainLinks}
            </Group>
          </Box>
        </AppShell.Header>
      )}
      <AppShell.Navbar>
        <AppShell.Section className={classes.header}>
          <Stack>
            <Group justify="space-between">
              <Group gap="xs" justify="left">
                <IconBroadcast stroke={3} className={basicInfo.isOnline ? classes.onlineIcon : classes.offlineIcon} />
                <Text lineClamp={1}>{basicInfo.title ?? 'Zhang Accounting'}</Text>
              </Group>
              <ActionIcon variant="white" color="gray" size="sm" onClick={sendReloadEvent}>
                <IconRefresh size="1.125rem" />
              </ActionIcon>
            </Group>
            <TextInput placeholder="Search" size="xs" leftSectionPointerEvents="none" leftSection={<IconSearch size={12} stroke={1.5} />} />
            <NewTransactionButton />
          </Stack>
        </AppShell.Section>

        {/*<Navbar.Section grow className={classes.section} >*/}
        <AppShell.Section grow className={classes.section} mx="sm">
          <div className={classes.mainLinks}>
            <UnstyledButton
              component={RouteLink}
              to={'/'}
              key={'NAV_HOME'}
              className={`${classes.mainLink} ${matchPath(location.pathname, '/') ? classes.activeMainLink : ''}`}
            >
              <div className={classes.mainLinkInner}>
                <IconSmartHome size={20} className={classes.mainLinkIcon} stroke={1.5} />
                <span>{t('NAV_HOME')}</span>
              </div>
              {errorsCount > 0 && (
                <Badge size="sm" color="pink" variant="filled" className={classes.mainLinkBadge}>
                  {errorsCount}
                </Badge>
              )}
            </UnstyledButton>
            {mainLinks}
          </div>
        </AppShell.Section>

        {basicInfo.updatableVersion && (
          <AppShell.Section className={classes.section}>
            <Group justify="center" gap={'sm'}>
              <Anchor href="https://zhang-accounting.kilerd.me/installation/4-upgrade/" target="_blank">
                🎉 New Version is available!
              </Anchor>
            </Group>
          </AppShell.Section>
        )}
      </AppShell.Navbar>

      <AppShell.Main>
        <Router />
      </AppShell.Main>
    </AppShell>
  );
}
