import { Box, Group, Stack, Text } from '@mantine/core';
import * as React from 'react';
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  card: {
    backgroundColor: 'transparent',
    padding: theme.spacing.sm,
    border: `2px solid ${theme.colors.gray[1]}`,
    borderRadius: theme.radius.sm,
    '&:hover': {
      border: `2px solid ${theme.colors[theme.primaryColor][6]}`,
    },
  },

  lead: {
    fontSize: theme.fontSizes.md,
    lineHeight: 1,
    color: theme.colors.gray[9],
    paddingBottom: theme.spacing.sm,
  },
}));

interface Props {
  title: string;
  rightSection?: React.ReactNode;
  children: React.ReactNode;
  noPadding?: boolean;
}

export default function Section({ children, title, rightSection }: Props) {
  const { classes } = useStyles();
  return (
    <Stack className={classes.card} mt="sm">
      <Group justify="space-between" className={classes.lead}>
        <Text fw={500}>{title}</Text>
        {rightSection}
      </Group>
      <Box>{children}</Box>
    </Stack>
  );
}
