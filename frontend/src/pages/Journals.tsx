import { useQuery } from '@apollo/client';
import { Container, Button, Grid, ScrollArea, Table, Title } from '@mantine/core';
import { useState } from 'react';
import JournalLine from '../components/JournalLine';
import JournalPreview from '../components/journalPreview/JournalPreview';
import { JouralListQuery, JournalItem, JOURNAL_LIST } from '../gql/jouralList';
function Journals() {
  const [page, setPage] = useState(1);
  const { loading, error, data, fetchMore } = useQuery<JouralListQuery>(JOURNAL_LIST, { variables: { page: 1 } });
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | undefined>(undefined);
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  const onMoreButtonClick = () => {
    fetchMore({
      variables: {
        page: page + 1,
      },
    });
    setPage(page + 1);
  };
  return (
    <Grid>
      <Grid.Col span={6}>
        <ScrollArea style={{ height: '96vh' }} offsetScrollbars>
          <Title order={2}>{data?.journals.data.length} Journals</Title>
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Date</th>
                <th style={{}}>Payee & Narration</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {data?.journals.data.map((journal, idx) => (
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
