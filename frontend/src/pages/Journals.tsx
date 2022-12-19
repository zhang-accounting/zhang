import {Grid, Pagination, ScrollArea, Table, Title} from '@mantine/core';
import {useLocalStorage, useMediaQuery} from '@mantine/hooks';
import {useState} from 'react';
import useSWR from 'swr';
import {fetcher} from '..';
import JournalLine from '../components/JournalLine';
import JournalPreview from '../components/journalPreview/JournalPreview';
import {JournalItem, Pageable} from '../rest-model';

function Journals() {
  const [layout] = useLocalStorage({ key: `journal-list-layout`, defaultValue: 'Smart' });
  const isWeb = useMediaQuery('(min-width: 768px)');
  const [page, setPage] = useState(1);
  const { data, error } = useSWR<Pageable<JournalItem>>(`/api/journals?page=${page}`, fetcher);
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | undefined>(undefined);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;
  const {total_page, records, current_page}  = data;

  const journalItems = (
    <>
      <Title order={2}>{records.length} Journals</Title>
      <Table verticalSpacing="xs" highlightOnHover>
        <thead>
          <tr>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {records.map((journal) => (
            <JournalLine key={journal.id} data={journal} onClick={setSelectedJournal} />
          ))}
        </tbody>
      </Table>
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
        <div>table view</div>
      )}
    </>
  );
}

export default Journals;
