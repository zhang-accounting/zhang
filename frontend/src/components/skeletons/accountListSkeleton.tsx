import { Skeleton, Table } from '@mantine/core';

export function AccountListSkeleton() {
  return (
    <>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton height={20} width="10%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton height={20} width="20%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton height={20} width="40%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton height={20} width="10%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton height={20} width="20%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton height={20} width="40%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
    </>
  );
}
