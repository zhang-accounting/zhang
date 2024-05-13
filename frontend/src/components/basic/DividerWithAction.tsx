import { ActionIcon, Divider, Grid } from '@mantine/core';
import { ReactNode } from 'react';

interface Props {
  value: String;
  icon: ReactNode;
  onActionClick(): void;
}
export default function DividerWithAction({ value, icon, onActionClick }: Props) {
  return (
    <Grid justify="space-between" align="center">
      <Grid.Col span={11}>
        <Divider label={value} size="xs"></Divider>
      </Grid.Col>
      <Grid.Col span={1}>
        <ActionIcon  variant="white" onClick={onActionClick}>{icon}</ActionIcon>
      </Grid.Col>
    </Grid>
  );
}
