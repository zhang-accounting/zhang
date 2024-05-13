import { Group, px } from '@mantine/core';
import { ReactNode } from 'react';
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  spoiled: {
    '& + &': {
      borderTop: `1px dashed ${theme.colors.dark[0]}`,
      marginTop: `calc(${theme.spacing.xs} * 0.5)`,
      marginBottom: `calc(${theme.spacing.xs} * 0.5)`,
    },
  },
}));

interface Props {
  children: ReactNode;
}

export default function DashLine({ children }: Props) {
  const { classes } = useStyles();
  return (
    <Group className={classes.spoiled} justify="space-between">
      {children}
    </Group>
  );
}
