import { useQuery } from '@apollo/client';
import { Button, Grid, ScrollArea, Table, Title } from '@mantine/core';
import { useEffect, useState } from 'react';
import JournalLine from '../components/JournalLine';
import JournalPreview from '../components/journalPreview/JournalPreview';
import { JouralListQuery, JournalItem, JOURNAL_LIST } from '../gql/jouralList';
function Journals() {
  const [existedData, setExistedData] = useState<{ [page: string]: JournalItem[] }>({});

  const [page, setPage] = useState(1);

  const { loading, error, data } = useQuery<JouralListQuery>(JOURNAL_LIST, { variables: { page: page } });
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | undefined>(undefined);
  useEffect(() => {
    if (data?.journals) {
      setExistedData((olddata) => {
        const newExitedData = { ...olddata };
        newExitedData[data?.journals.pageInfo.page.toString()] = data?.journals.data || [];
        return newExitedData;
      });
    }
  }, [data, loading, error]);

  const onMoreButtonClick = () => {
    setPage(page + 1);
  };
  const journals = Object.keys(existedData)
    .map((page) => parseInt(page))
    .sort()
    .flatMap((page: number) => existedData[page]);

  return (
    <Grid>
      <Grid.Col span={6}>
        <Title order={2}>{journals.length} Journals</Title>
        <ScrollArea style={{ height: '90vh' }} offsetScrollbars type="always">
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Date</th>
                <th style={{}}>Payee & Narration</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {journals.map((journal, idx) => (
                <JournalLine key={idx} data={journal} onClick={setSelectedJournal} />
              ))}
            </tbody>
          </Table>
          <Button onClick={onMoreButtonClick}>Fetch More</Button>
        </ScrollArea>
      </Grid.Col>
      <Grid.Col span={6}>
        <JournalPreview data={selectedJournal} />
      </Grid.Col>
    </Grid>
  );
}

export default Journals;
