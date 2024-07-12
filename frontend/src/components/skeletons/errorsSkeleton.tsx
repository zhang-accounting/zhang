import { Skeleton } from '@mantine/core';

export function ErrorsSkeleton() {
  return (
    <>
      <Skeleton height={20} radius="xs" />
      <Skeleton height={20} mt={10} radius="xs" />
      <Skeleton height={20} mt={10} radius="xs" />
      <Skeleton height={20} mt={10} radius="xs" />
    </>
  );
}
