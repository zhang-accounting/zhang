import { Route, Routes } from 'react-router-dom';
import Accounts from './pages/Accounts';
import Commodities from './pages/Commodities';
import Documents from './pages/Documents';
import Home from './pages/Home';
import Journals from './pages/Journals';
import RawEdit from './pages/RawEdit';
import Report from './pages/Report';
import Settings from './pages/Settings';
import SingleAccount from './pages/SingleAccount';
import SingleCommodity from './pages/SingleCommodity';

import { Link as RouteLink } from 'react-router-dom';
import NewTransactionButton from './components/NewTransactionButton';
import { createStyles, Navbar, TextInput, Code, UnstyledButton, Badge, Text, Group } from '@mantine/core';
import { IconBulb, IconUser, IconCheckbox, IconSearch, IconReceipt2, TablerIcon } from '@tabler/icons';

import { AppShell, Grid } from '@mantine/core';

const useStyles = createStyles((theme) => ({
  header: {
    padding: theme.spacing.md,
    paddingTop: 0,
    marginLeft: -theme.spacing.md,
    marginRight: -theme.spacing.md,
    color: theme.colorScheme === 'dark' ? theme.white : theme.black,
    borderBottom: `1px solid ${theme.colorScheme === 'dark' ? theme.colors.dark[4] : theme.colors.gray[3]}`,
  },
  navbar: {
    paddingTop: 0,
  },

  section: {
    marginLeft: -theme.spacing.md,
    marginRight: -theme.spacing.md,
    marginBottom: theme.spacing.md,

    '&:not(:last-of-type)': {
      borderBottom: `1px solid ${theme.colorScheme === 'dark' ? theme.colors.dark[4] : theme.colors.gray[3]}`,
    },
  },

  searchCode: {
    fontWeight: 700,
    fontSize: 10,
    backgroundColor: theme.colorScheme === 'dark' ? theme.colors.dark[7] : theme.colors.gray[0],
    border: `1px solid ${theme.colorScheme === 'dark' ? theme.colors.dark[7] : theme.colors.gray[2]}`,
  },

  mainLinks: {
    paddingLeft: theme.spacing.md - theme.spacing.xs,
    paddingRight: theme.spacing.md - theme.spacing.xs,
    paddingBottom: theme.spacing.md,
  },

  mainLink: {
    display: 'flex',
    alignItems: 'center',
    width: '100%',
    fontSize: theme.fontSizes.sm,
    padding: `8px ${theme.spacing.xs}px`,
    borderRadius: theme.radius.sm,
    fontWeight: 500,
    color: theme.colorScheme === 'dark' ? theme.colors.dark[0] : theme.colors.gray[7],

    '&:hover': {
      backgroundColor: theme.colorScheme === 'dark' ? theme.colors.dark[6] : theme.colors.gray[0],
      color: theme.colorScheme === 'dark' ? theme.white : theme.black,
    },
  },

  mainLinkInner: {
    display: 'flex',
    alignItems: 'center',
    flex: 1,
  },

  mainLinkIcon: {
    marginRight: theme.spacing.sm,
    color: theme.colorScheme === 'dark' ? theme.colors.dark[2] : theme.colors.gray[6],
  },

  mainLinkBadge: {
    padding: 0,
    width: 20,
    height: 20,
    pointerEvents: 'none',
  },

  collections: {
    paddingLeft: theme.spacing.md - 6,
    paddingRight: theme.spacing.md - 6,
    paddingBottom: theme.spacing.md,
  },

  collectionsHeader: {
    paddingLeft: theme.spacing.md + 2,
    paddingRight: theme.spacing.md,
    marginBottom: 5,
  },

  collectionLink: {
    display: 'block',
    padding: `8px ${theme.spacing.xs}px`,
    textDecoration: 'none',
    borderRadius: theme.radius.sm,
    fontSize: theme.fontSizes.xs,
    color: theme.colorScheme === 'dark' ? theme.colors.dark[0] : theme.colors.gray[7],
    lineHeight: 1,
    fontWeight: 500,

    '&:hover': {
      backgroundColor: theme.colorScheme === 'dark' ? theme.colors.dark[6] : theme.colors.gray[0],
      color: theme.colorScheme === 'dark' ? theme.white : theme.black,
    },
  },
}));

interface LinkItem {
  icon: TablerIcon;
  label: string;
  uri: string;
  notifications?: number;
}

const links: LinkItem[] = [
  { icon: IconBulb, label: 'Home', uri: '/' },
  { icon: IconCheckbox, label: 'Journals', uri: '/journals' },
  { icon: IconUser, label: 'Accounts', uri: '/accounts' },
  { icon: IconUser, label: 'Commodities', uri: '/commodities' },
  { icon: IconUser, label: 'Documents', uri: '/documents' },
  { icon: IconUser, label: 'Report', uri: '/report' },
  { icon: IconUser, label: 'Liability', uri: '/liability' },
  { icon: IconUser, label: 'Editor', uri: '/edit' },
  { icon: IconUser, label: 'Settings', uri: '/settings' },
];

// const collections = [
//   { emoji: '👍', label: 'Sales' },
//   { emoji: '🚚', label: 'Deliveries' },
//   { emoji: '💸', label: 'Discounts' },
//   { emoji: '💰', label: 'Profits' },
//   { emoji: '✨', label: 'Reports' },
//   { emoji: '🛒', label: 'Orders' },
//   { emoji: '📅', label: 'Events' },
//   { emoji: '🙈', label: 'Debts' },
// ];

export default function App() {
  const { classes } = useStyles();

  const mainLinks = links.map((link) => (
    <UnstyledButton component={RouteLink} to={link.uri} key={link.label} className={classes.mainLink}>
      <div className={classes.mainLinkInner}>
        <link.icon size={20} className={classes.mainLinkIcon} stroke={1.5} />
        <span>{link.label}</span>
      </div>
      {link.notifications && (
        <Badge size="sm" variant="filled" className={classes.mainLinkBadge}>
          {link.notifications}
        </Badge>
      )}
    </UnstyledButton>
  ));

  // const collectionLinks = collections.map((collection) => (
  //   <a href="/" onClick={(event) => event.preventDefault()} key={collection.label} className={classes.collectionLink}>
  //     <span style={{ marginRight: 9, fontSize: 16 }}>{collection.emoji}</span> {collection.label}
  //   </a>
  // ));

  return (
    <AppShell
      padding="md"
      navbar={
        <Navbar width={{ sm: 300 }} p="md" className={classes.navbar}>
          <Navbar.Section className={classes.header}>
            <Group position="apart">
              <Group spacing="xs" position="left">
                <IconReceipt2 stroke={1.5} />
                <Text>ZHANG</Text>
              </Group>
              <Code sx={{ fontWeight: 700 }}>v0.1.1</Code>
            </Group>
          </Navbar.Section>

          <Grid>
            <Grid.Col span={8} py="md">
              <TextInput placeholder="Search" size="xs" icon={<IconSearch size={12} stroke={1.5} />} />
            </Grid.Col>
            <Grid.Col span={4} py="md">
              <NewTransactionButton />
            </Grid.Col>
          </Grid>

          <Navbar.Section className={classes.section}>
            <div className={classes.mainLinks}>{mainLinks}</div>
          </Navbar.Section>

          {/* <Navbar.Section grow className={classes.section}>
            <Group className={classes.collectionsHeader} position="apart">
              <Text size="xs" weight={500} color="dimmed">
                Collections
              </Text>
              <Tooltip label="Create collection" withArrow position="right">
                <ActionIcon variant="default" size={18}>
                  <IconPlus size={12} stroke={1.5} />
                </ActionIcon>
              </Tooltip>
            </Group>
            <div className={classes.collections}>{collectionLinks}</div>
          </Navbar.Section> */}
        </Navbar>
      }>
      <Routes>
        <Route path="/" element={<Home />} />
        <Route path="journals" element={<Journals />} />
        <Route path="accounts" element={<Accounts />} />
        <Route path="/accounts/:accountName" element={<SingleAccount />} />
        <Route path="/commodities" element={<Commodities />} />
        <Route path="/commodities/:commodityName" element={<SingleCommodity />} />
        <Route path="documents" element={<Documents />} />
        <Route path="/edit" element={<RawEdit />} />
        <Route path="/report" element={<Report />} />
        <Route path="/settings" element={<Settings />} />
      </Routes>
    </AppShell>
  );
}
