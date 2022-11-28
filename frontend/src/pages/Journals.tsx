import { Button, Grid, ScrollArea, Table, Title } from '@mantine/core';
import { useLocalStorage, useMediaQuery } from '@mantine/hooks';
import { useState } from 'react';
import useSWR from 'swr';
import { fetcher } from '..';
import JournalLine from '../components/JournalLine';
import JournalPreview from '../components/journalPreview/JournalPreview';
import { JournalItem } from '../rest-model';

function Journals() {
  const [existedData, setExistedData] = useState<{ [page: string]: JournalItem[] }>({});
  const [layout, setLayout] = useLocalStorage({ key: `journal-list-layout`, defaultValue: 'Smart' });
  const isWeb = useMediaQuery('(min-width: 768px)');
  const [page, setPage] = useState(1);
  const { data, error } = useSWR<JournalItem[]>(`/api/journals`, fetcher);
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | undefined>(undefined);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  const onMoreButtonClick = () => {
    setPage(page + 1);
  };
  const journalItems = (
    <>
      <Title order={2}>{data.length} Journals</Title>
      <Table verticalSpacing="xs" highlightOnHover>
        <thead>
          <tr>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {data.map((journal) => (
            <JournalLine key={journal.id} data={journal} onClick={setSelectedJournal} />
          ))}
        </tbody>
      </Table>
      <Button onClick={onMoreButtonClick}>Fetch More</Button>
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
