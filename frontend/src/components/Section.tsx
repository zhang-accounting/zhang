import { Card, Group, Text } from '@mantine/core';
import React from 'react';

interface Props {
  title: string;
  rightSection?: React.ReactNode;
  children: React.ReactNode;
}
export default function Section({ children, title, rightSection }: Props) {
  return (
    <Card withBorder shadow="sm" radius="sm" mt="sm">
      <Card.Section withBorder inheritPadding py="xs">
        <Group position="apart">
          <Text weight={500}>{title}</Text>
          {rightSection}
        </Group>
      </Card.Section>
      <Card.Section p="md">{children}</Card.Section>
    </Card>
  );
}
