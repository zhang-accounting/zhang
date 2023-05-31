import { Group, Text, createStyles } from '@mantine/core';

const useStyles = createStyles((theme) => ({
  payee: {
    fontWeight: 700,
    color: theme.colorScheme === 'dark' ? theme.white : theme.black,
    '&:after': {
      fontWeight: 700,
      marginLeft: theme.spacing.xs,
      content: '"·"',
    },
  },
  narration: {},
}));

interface Props {
  payee?: string;
  narration?: string;
}
export default function PayeeNarration(props: Props) {
  const { classes } = useStyles();
  return (
    <Group spacing="xs">
      {props.payee && <Text className={classes.payee}>{props.payee}</Text>}
      <Text className={classes.narration}>{props.narration ?? ''}</Text>
    </Group>
  );
}
