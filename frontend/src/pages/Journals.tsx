import { useQuery } from '@apollo/client';
import { Button, Grid, ScrollArea, Table, Title } from '@mantine/core';
import { useEffect, useState } from 'react';
import JournalLine from '../components/JournalLine';
import JournalPreview from '../components/journalPreview/JournalPreview';
import { JouralListQuery, JournalItem, JOURNAL_LIST } from '../gql/jouralList';
function Journals() {
  const [existedData, setExistedData] = useState<JournalItem[]>([]);

  const [page, setPage] = useState(1);

  const { loading, error, data } = useQuery<JouralListQuery>(JOURNAL_LIST, { variables: { page: page } });
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | undefined>(undefined);
  useEffect(() => {
    if (loading) return;
    if (error) return;
    setExistedData([...existedData, ...(data?.journals.data || [])]);
  }, [data]);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  const onMoreButtonClick = () => {
    setPage(page + 1);
  };
  console.log('journal', data);
  return (
    <Grid>
      <Grid.Col span={6}>
        <ScrollArea style={{ height: '96vh' }} offsetScrollbars>
          <Title order={2}>{existedData.length} Journals</Title>
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Date</th>
                <th style={{}}>Payee & Narration</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {existedData.map((journal, idx) => (
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
