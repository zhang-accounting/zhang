import { Container, Grid, Title } from '@mantine/core';
import useSWR from 'swr';
import { fetcher } from '..';
import SingleFileEdit from '../components/SingleFileEdit';
import { TableOfContentsFloating, Tier, ZHANG_VALUE } from '../components/basic/TableOfContentsFloating';
import { useState } from 'react';

function RawEdit() {
  const { data, error } = useSWR<string[]>('/api/files', fetcher);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  const tree: Tier = {};
  data
    .map((it) => it.replace(/^\/|\/$/g, ''))
    .forEach((entry) => {
      let ref_tree = tree;
      entry.split(/[/\\]/).forEach((path) => {
        if (!ref_tree.hasOwnProperty(path)) {
          ref_tree[path] = {};
        }
        ref_tree = ref_tree[path];
      });
      ref_tree[ZHANG_VALUE] = entry;
    });
  return (
    <Container fluid>
      <Grid>
        <Grid.Col span={2}>
          {' '}
          <Title order={4}>Nav</Title>
          <TableOfContentsFloating files={tree} onChange={(value) => setSelectedFile(value)} />
        </Grid.Col>
        <Grid.Col span={10}>
          <Title order={4}>File:{selectedFile}</Title>
          {selectedFile && <SingleFileEdit name={selectedFile} path={selectedFile} />}
        </Grid.Col>
      </Grid>
    </Container>
  );
}

export default RawEdit;
