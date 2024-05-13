import { PluginResponse } from '../rest-model';
import { Badge, Group, Stack, Text } from '@mantine/core';
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  card: {
    backgroundColor: 'transparent',
    padding: theme.spacing.sm,
    border: `2px solid ${theme.colors.gray[1]}`,
    borderRadius: theme.radius.sm,
  },
}));

interface Props extends PluginResponse {
}

export default function PluginBox(props: Props) {
  const { classes } = useStyles();
  return (
    <Stack className={classes.card} gap={'xs'}>
      <Group justify={'apart'}>
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
