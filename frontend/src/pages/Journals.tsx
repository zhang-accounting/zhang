import { Button, Group, Pagination, Table } from '@mantine/core';
import { format } from 'date-fns';
import { groupBy } from 'lodash';
import { useEffect } from 'react';
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchJournals, journalsSlice } from '../states/journals';
import { Heading } from '../components/basic/Heading';
import { useTranslation } from 'react-i18next';

function Journals() {
  const { t } = useTranslation();
  const { current_page, status: journalStatus, items, total_number, total_page } = useAppSelector((state) => state.journals);
  const dispatch = useAppDispatch();
  useEffect(() => {
    if (journalStatus === LoadingState.NotReady) {
      dispatch(fetchJournals());
    }
  }, [dispatch, journalStatus]);

  const onPage = (page: number) => {
    dispatch(journalsSlice.actions.setPage({ current_page: page }));
    dispatch(fetchJournals());
  };

  if (journalStatus === LoadingState.Loading || journalStatus === LoadingState.NotReady) return <>loading</>;

  const groupedRecords = groupBy(items, (record) => format(new Date(record.datetime), 'yyyy-MM-dd'));

  return (
    <>
      <Heading title={`${total_number} Journals`}></Heading>
      <Group my="lg" px="sm">
        <Button variant="outline" color="gray" radius="xl" size="xs" onClick={() => dispatch(fetchJournals())}>
          {t('REFRESH')}
        </Button>
      </Group>
      <Table verticalSpacing="xs" highlightOnHover withBorder>
        <thead>
          <tr>
            <th style={{ width: '200px' }}>Date</th>
            <th>Type</th>
            <th>Payee</th>
            <th>Narration</th>
            <th style={{ textAlign: 'right' }}>Amount</th>
            <th style={{ textAlign: 'right' }}>Operation</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(groupedRecords).map((entry) => {
            return (
              <>
                {entry[1].map((journal) => (
                  <TableViewJournalLine key={journal.id} data={journal} />
                ))}
              </>
            );
          })}
        </tbody>
      </Table>
      <Pagination my="xs" total={total_page} value={current_page} onChange={onPage} position="center" />
    </>
  );
}

export default Journals;
