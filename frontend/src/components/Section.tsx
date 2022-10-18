import { Card, Group, Text } from '@mantine/core';
import React from 'react';

interface Props {
  title: string;
  children: React.ReactNode;
}
export default function Section({ children, title }: Props) {
  return (
    <Card withBorder shadow="sm" radius="sm" mb="sm">
      <Card.Section withBorder inheritPadding py="xs">
        <Group position="apart">
          <Text weight={500}>{title}</Text>
        </Group>
      </Card.Section>
      <Card.Section p="md">{children}</Card.Section>
    </Card>
  );
}
