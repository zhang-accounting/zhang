import { Box, createStyles, Group, px, Stack, Text } from '@mantine/core';
import React from 'react';

const useStyles = createStyles((theme) => ({
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
    fontSize: px(theme.fontSizes.md),
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
      <Group position="apart" className={classes.lead}>
        <Text weight={500}>{title}</Text>
        {rightSection}
      </Group>
      <Box>{children}</Box>
    </Stack>
  );
}
