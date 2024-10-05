import { Skeleton } from '@mantine/core';
import { TableRow, TableCell } from '../ui/table';

export function JournalListSkeleton() {
  return (
    <>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} width="30%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} width="30%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} width="30%" mt={10} radius="xs" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton height={20} mt={10} radius="xs" />
        </TableCell>
      </TableRow>
    </>
  );
}
