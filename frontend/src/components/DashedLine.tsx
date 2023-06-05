import { createStyles, Group, px } from '@mantine/core';
import { ReactNode } from 'react';
const useStyles = createStyles((theme) => ({
  spoiled: {
    '& + &': {
      borderTop: `1px dashed ${theme.colors.dark[0]}`,
      marginTop: px(theme.spacing.xs) * 0.5,
      marginBottom: px(theme.spacing.xs) * 0.5,
    },
  },
}));
interface Props {
  children: ReactNode;
}

export default function DashLine({ children }: Props) {
  const { classes } = useStyles();
  return (
    <Group className={classes.spoiled} position="apart">
      {children}
    </Group>
  );
}
