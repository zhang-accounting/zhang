import { Code, Container, Grid, Highlight, ScrollArea, SimpleGrid, Textarea } from '@mantine/core';
import { useState } from 'react';
import { useLocalStorage } from '@mantine/hooks';

export default function WechatExporter() {
  const [config, setConfig] = useLocalStorage({ key: 'wechat-extractor-config', defaultValue: '' });
  const [res] = useState('');
  return (
    <Container fluid>
      <SimpleGrid cols={2} spacing="md">
        <Grid gutter="md">
          <Grid.Col>
            {/*<DropzoneButton*/}
            {/*  gql={WECHAT_EXTRACTOR}*/}
            {/*  variables={{ config: config }}*/}
            {/*  single*/}
            {/*  onResult={(result) => {*/}
            {/*    setRes(result.data.res);*/}
            {/*  }}*/}
            {/*/>*/}
          </Grid.Col>
          <Grid.Col>
            <Textarea value={config} onChange={(event) => setConfig(event.target.value)} />
          </Grid.Col>
        </Grid>
        <ScrollArea style={{ height: 'calc(100vh - 2 * var(--mantine-spacing-xs, 16px))' }} offsetScrollbars type="always">
          <Code block>
            <Highlight highlight="Expenses:FixMe">{res}</Highlight>
          </Code>
        </ScrollArea>
      </SimpleGrid>
    </Container>
  );
}
