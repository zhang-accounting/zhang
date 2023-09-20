import { Group, Title } from '@mantine/core';
import { ReactElement } from 'react';

interface Props {
  title: string;
  elements?: ReactElement | ReactElement[];
  rightSection?: ReactElement;
}

export function Heading(props: Props) {
  return (
    <Group position="apart" py="md" px="xs" align="baseline">
      <Title order={2}>{props.title}</Title>
      {props.rightSection}
    </Group>
  );
}
