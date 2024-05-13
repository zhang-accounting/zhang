import { Button, CloseButton, Group, Input, Pagination, Table, Text } from '@mantine/core';
import { format } from 'date-fns';
import { groupBy } from 'lodash-es';
import { useEffect, useState } from 'react';
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchJournals, journalsSlice } from '../states/journals';
import { Heading } from '../components/basic/Heading';
import { useTranslation } from 'react-i18next';
import { useDebouncedValue, useDocumentTitle } from '@mantine/hooks';
import { IconFilter } from '@tabler/icons-react';
import { JournalListSkeleton } from '../components/skeletons/journalListSkeleton';

function Journals() {
  const { t } = useTranslation();
  const [filter, setFilter] = useState('');
  const [debouncedFilter] = useDebouncedValue(filter, 200);
  const { current_page, status: journalStatus, items, total_number, total_page } = useAppSelector((state) => state.journals);
  const dispatch = useAppDispatch();
  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`Journals - ${ledgerTitle}`);

  useEffect(() => {
    if (journalStatus === LoadingState.NotReady) {
      dispatch(fetchJournals(debouncedFilter));
    }
  }, [dispatch, journalStatus, debouncedFilter]);

  useEffect(() => {
    dispatch(fetchJournals(debouncedFilter));
  }, [dispatch, debouncedFilter]);

  const onPage = (page: number) => {
    dispatch(journalsSlice.actions.setPage({ current_page: page }));
    dispatch(fetchJournals(debouncedFilter));
  };

  const groupedRecords = groupBy(items, (record) => format(new Date(record.datetime), 'yyyy-MM-dd'));

  return (
    <>
      <Heading title={`${total_number} Journals`}></Heading>
      <Group my="lg" px="sm">
        <Button variant="outline" color="gray" radius="xl" size="xs" onClick={() => dispatch(fetchJournals(filter))}>
          {t('REFRESH')}
        </Button>
        <Input
          leftSection={<IconFilter size="1rem" />}
          placeholder={t('ACCOUNT_FILTER_PLACEHOLDER')}
          value={filter}
          onChange={(event: any) => setFilter(event.currentTarget.value)}
          rightSection={<CloseButton aria-label={t('ACCOUNT_FILTER_CLOSE_BUTTON_ARIA')} onClick={() => setFilter('')} />}
        />
      </Group>
      <Table verticalSpacing="xs" withTableBorder>
        <Table.Thead>
          <Table.Tr>
            <Table.Th style={{ width: '100px' }}>Date</Table.Th>
            <Table.Th style={{ width: '10px' }}>Type</Table.Th>
            <Table.Th>Payee · Narration</Table.Th>
            <Table.Th style={{ textAlign: 'right' }}>Amount</Table.Th>
            <Table.Th style={{ textAlign: 'right' }}>Operation</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {(journalStatus === LoadingState.Loading || journalStatus === LoadingState.NotReady) && <JournalListSkeleton />}
          {journalStatus === LoadingState.Success &&
            Object.entries(groupedRecords).map((entry) => {
              return (
                <>
                  <Table.Tr>
                    <Table.Td colSpan={6}>
                      <Text c={'dimmed'} size={'sm'}>
                        {entry[0]}
                      </Text>
                    </Table.Td>
                  </Table.Tr>
                  {entry[1].map((journal) => (
                    <TableViewJournalLine key={journal.id} data={journal} />
                  ))}
                </>
              );
            })}
        </Table.Tbody>
      </Table>

      <Group justify={'center'}>
        <Pagination my="xs" total={total_page} value={current_page} onChange={onPage} />
      </Group>
    </>
  );
}

export default Journals;
