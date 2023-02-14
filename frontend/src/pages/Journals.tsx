import { Button, Grid, Group, Pagination, ScrollArea, Table, Title } from '@mantine/core';
import { useLocalStorage, useMediaQuery } from '@mantine/hooks';
import { IconLayout2, IconListDetails } from '@tabler/icons';
import { useState } from 'react';
import useSWR from 'swr';
import { fetcher } from '..';
import JournalPreview from '../components/journalPreview/JournalPreview';
import { JournalItem, Pageable } from '../rest-model';
import { groupBy } from 'lodash';
import { format } from 'date-fns'
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';
import JournalLine from '../components/journalLines/smartView/JournalLine';

function Journals() {
  const [layout, setLayout] = useLocalStorage({ key: `journal-list-layout`, defaultValue: 'Smart' });
  const isWeb = useMediaQuery('(min-width: 768px)');
  const [page, setPage] = useState(1);
  const { data, error } = useSWR<Pageable<JournalItem>>(`/api/journals?page=${page}`, fetcher);
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | undefined>(undefined);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;
  const { total_page, records, current_page } = data;
  const groupedRecords = groupBy(records, record => format(new Date(record.datetime), 'yyyy-MM-dd'));

  const header = <Group position="apart" my="sm">
    <Title order={2}>{records.length} Journals</Title>
    <Button.Group>
      <Button disabled={layout === 'Smart'} leftIcon={<IconLayout2 size={14} />} variant="default" onClick={() => setLayout('Smart')}>
        Smart
      </Button>
      <Button disabled={layout === 'Table'} leftIcon={<IconListDetails size={14} />} variant="default" onClick={() => setLayout('Table')}>
        Table
      </Button>
    </Button.Group>
  </Group>
  const journalItems = (
    <>
      {header}

      {Object.entries(groupedRecords).map((entry) => {
        const date = entry[0];
        const records = entry[1];
        console.log(date, records);

        return <>
          <Table verticalSpacing="xs" highlightOnHover mt="xs">
            <thead>
              <tr><th>{date}</th></tr>
            </thead>
            <tbody>
              {records.map((journal) => (
                <JournalLine key={journal.id} data={journal} onClick={setSelectedJournal} />
              ))}
            </tbody>
          </Table>
        </>
      })}
      <Pagination mt="xs" total={total_page} page={current_page} onChange={setPage} position="center" />
    </>
  );
  return (
    <>
      {layout === 'Smart' ? (
        <Grid>
          {isWeb ? (
            <>
              <Grid.Col span={6}>
                <ScrollArea style={{ height: 'calc(100vh - 2 * var(--mantine-spacing-xs, 16px))' }} offsetScrollbars type="always">
                  {journalItems}
                </ScrollArea>
              </Grid.Col>
              <Grid.Col span={6}>
                <ScrollArea style={{ height: 'calc(100vh - 2 * var(--mantine-spacing-xs, 16px))' }}>
                  <JournalPreview data={selectedJournal} />
                </ScrollArea>
              </Grid.Col>
            </>
          ) : (
            <>{journalItems}</>
          )}
        </Grid>
      ) : (

        <>
          {header}
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Date</th>
                <th>Type</th>
                <th>Payee</th>
                <th>Narration</th>
                <th>Amount</th>
                <th>Operation</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(groupedRecords).map((entry) => {
                return <>
                      {entry[1].map((journal) => (
                        <TableViewJournalLine key={journal.id} data={journal} />
                      ))}
                </>
              })}
            </tbody>
          </Table>
          <Pagination my="xs" total={total_page} page={current_page} onChange={setPage} position="center" />
   
        </>
      )}
    </>
  );
}

export default Journals;
