import { PluginResponse } from '../rest-model';
import { Badge, createStyles, Group, Stack, Text } from '@mantine/core';

const useStyles = createStyles((theme) => ({
  card: {
    backgroundColor: 'transparent',
    padding: theme.spacing.sm,
    border: `2px solid ${theme.colors.gray[1]}`,
    borderRadius: theme.radius.sm,
  },
}));

interface Props extends PluginResponse {}

export default function PluginBox(props: Props) {
  const { classes } = useStyles();
  return (
    <Stack className={classes.card} spacing={'xs'}>
      <Group position={'apart'}>
        <Text>{props.name}</Text>
        <Badge variant={'filled'}>{props.version}</Badge>
      </Group>
      <Group>
        {props.plugin_type.map((item) => (
          <Badge>{item}</Badge>
        ))}
      </Group>
    </Stack>
  );
}
