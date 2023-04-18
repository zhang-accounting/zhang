import { Box } from '@mantine/core';
import { ReactElement } from 'react';
interface Props {
  title?: string;
  children: ReactElement | ReactElement[];
}
export default function Block({ title, children }: Props) {
  return (
    <Box px={4} py={2}>
      {title && <Box mb={2}>{title}</Box>}
      <Box>{children}</Box>
    </Box>
  );
}
