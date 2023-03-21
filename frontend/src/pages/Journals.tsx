import { Pagination, Table, Title } from '@mantine/core';
import { format } from 'date-fns';
import { groupBy } from 'lodash';
import { useEffect } from 'react';
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchJournals, journalsSlice } from '../states/journals';

function Journals() {
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
      <Title order={2}>{total_number} Journals</Title>
      <Table verticalSpacing="xs" highlightOnHover>
        <thead>
          <tr>
            <th>Date</th>
            <th>Type</th>
            <th>Payee</th>
            <th>Narration</th>
            <th style={{textAlign:"right"}}>Amount</th>
            <th>Operation</th>
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
      <Pagination my="xs" total={total_page} page={current_page} onChange={onPage} position="center" />
    </>
  );
}

export default Journals;
