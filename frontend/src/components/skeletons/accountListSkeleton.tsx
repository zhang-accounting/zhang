import { Skeleton, Table, Text } from '@mantine/core';

export function AccountListSkeleton() {
  return (
    <>
      <Table.Tr>
        <Table.Td colSpan={2}>
          <Skeleton height={20} width="10%" mt={10} radius="xs" />
        </Table.Td>
      </Table.Tr>
      <Table.Tr>
        <Table.Td colSpan={2}>
          <Skeleton height={20} width="20%" mt={10} radius="xs" />
        </Table.Td>
      </Table.Tr>
      <Table.Tr>
        <Table.Td colSpan={2}>
          <Skeleton height={20} width="40%" mt={10} radius="xs" />
        </Table.Td>
      </Table.Tr>
      <Table.Tr>
        <Table.Td colSpan={2}>
          <Skeleton height={20} width="10%" mt={10} radius="xs" />
        </Table.Td>
      </Table.Tr>
      <Table.Tr>
        <Table.Td colSpan={2}>
          <Skeleton height={20} width="20%" mt={10} radius="xs" />
        </Table.Td>
      </Table.Tr>
      <Table.Tr>
        <Table.Td colSpan={2}>
          <Skeleton height={20} width="40%" mt={10} radius="xs" />
        </Table.Td>
      </Table.Tr>
    </>
  );
}
