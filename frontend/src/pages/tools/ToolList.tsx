import { Box, Container, SimpleGrid, Text, Title, UnstyledButton } from '@mantine/core';
import { createStyles } from '@mantine/emotion';
import { IconRelationOneToMany } from '@tabler/icons-react';
import { Link as RouteLink } from 'react-router-dom';

const toolItems = [
  {
    title: 'Batch Accounts Balance',
    icon: IconRelationOneToMany,
    color: 'cyan',
    uri: '/tools/batch-balance',
  },
];

const useStyles = createStyles((theme, _, u) => ({
  item: {
    [u.dark]: {
      backgroundColor: theme.colors.dark[7],
    },
    [u.light]: {
      backgroundColor: theme.colors.gray[0],

    },
    borderRadius: theme.radius.md,
    transition: 'box-shadow 150ms ease, transform 100ms ease',
    position: 'relative',
    '&:after': {
      content: '" "',
      display: 'block',
      paddingBottom: '100%',
    },
    '&:hover': {
      boxShadow: `${theme.shadows.md} !important`,
      transform: 'scale(1.05)',
    },
  },
  itemContent: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: '100%',
    height: '100%',
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    textAlign: 'center',
  },
}));
export default function ToolList() {
  const { classes, theme } = useStyles();
  const items = toolItems.map((item) => (
    <UnstyledButton component={RouteLink} to={item.uri} key={item.title} className={classes.item}>
      <Box className={classes.itemContent}>
        <item.icon color={theme.colors[item.color][6]} size={32} />
        <Text size="xs" mt={7}>
          {item.title}
        </Text>
      </Box>
    </UnstyledButton>
  ));

  return (
    <Container fluid>
      <Title order={2}>Tools</Title>
      <SimpleGrid cols={6} mt="md">
        {items}
      </SimpleGrid>
    </Container>
  );
}
