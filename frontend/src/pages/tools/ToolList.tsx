import { Container, Title, SimpleGrid, UnstyledButton, Text, createStyles } from '@mantine/core';
import { IconPackgeExport } from '@tabler/icons';
import { Link as RouteLink } from 'react-router-dom';

const toolItems = [
    { title: 'Wechat Exporter', icon: IconPackgeExport, color: 'cyan', uri: '/tools/wechat-exporter' }
];

const useStyles = createStyles((theme) => ({
  item: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    textAlign: 'center',
    borderRadius: theme.radius.md,
    paddingTop: theme.spacing.lg,
    paddingBottom: 1.5 * theme.spacing.lg,
    backgroundColor: theme.colorScheme === 'dark' ? theme.colors.dark[7] : theme.colors.gray[0],
    transition: 'box-shadow 150ms ease, transform 100ms ease',

    '&:hover': {
      boxShadow: `${theme.shadows.md} !important`,
      transform: 'scale(1.05)',
    },
  },
}));
export default function ToolList() {
  const { classes, theme } = useStyles();
  const items = toolItems.map((item) => (
    <UnstyledButton component={RouteLink} to={item.uri} key={item.title} className={classes.item}>
      <item.icon color={theme.colors[item.color][6]} size={32} />
      <Text size="xs" mt={7}>
        {item.title}
      </Text>
    </UnstyledButton>
  ));

  return (
    <Container fluid>
      <Title order={2}>Tools</Title>
      <SimpleGrid cols={4} mt="md">
        {items}
      </SimpleGrid>
    </Container>
  );
}
