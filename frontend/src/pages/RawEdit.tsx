import SingleFileEdit from '../components/SingleFileEdit';
import { useQuery } from '@apollo/client';
import { Tabs } from '@mantine/core';
import { FileListQuery, FILE_LIST } from '../gql/fileList';
import { Container } from '@mantine/core';

function RawEdit() {
  const { loading, error, data } = useQuery<FileListQuery>(FILE_LIST);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  return (
    <Container fluid>
      <Tabs orientation="vertical">
        <Tabs.List>
          {data?.entries.map((entry) => (
            <Tabs.Tab value={entry.name}>{entry.name.split('/').pop()}</Tabs.Tab>
          ))}
        </Tabs.List>

        {data?.entries.map((entry) => (
          <Tabs.Panel value={entry.name} pt="xs">
            <SingleFileEdit name={entry.name.split('/').pop()} path={entry.name} />
          </Tabs.Panel>
        ))}
      </Tabs>
    </Container>
  );
}

export default RawEdit;
