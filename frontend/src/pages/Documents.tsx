import { Container, Group, SegmentedControl, SimpleGrid, Table, Title } from '@mantine/core';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { format } from 'date-fns';
import useSWR from 'swr';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { Document } from '../rest-model';
import { Heading } from '../components/basic/Heading';
import { groupBy, reverse, sortBy } from 'lodash-es';
import { TextBadge } from '../components/basic/TextBadge';
import { useNavigate } from 'react-router';
import { useState } from 'react';
import 'yet-another-react-lightbox/styles.css';
import { ImageLightBox } from '../components/ImageLightBox';
import { isDocumentAnImage } from '../utils/documents';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { fetcher } from '../global.ts';

export default function Documents() {
  let navigate = useNavigate();
  const [layout, setLayout] = useLocalStorage({ key: `document-list-layout`, defaultValue: 'Grid' });
  const { data: documents, error } = useSWR<Document[]>('/api/documents', fetcher);
  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Documents - ${ledgerTitle}`);

  if (error) return <div>failed to load</div>;
  if (!documents) return <div>loading...</div>;

  const groupedDocuments = reverse(
    sortBy(
      groupBy(documents, (document) => format(new Date(document.datetime), 'yyyy-MM')),
      (it) => it[0].datetime,
    ),
  );
  return (
    <Container fluid>
      <Group justify="space-between">
        <Heading title={`${documents.length} Documents`}></Heading>
        <SegmentedControl value={layout} onChange={setLayout} data={['Grid', 'Table']} />
      </Group>
      <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />
      {layout === 'Grid' ? (
        <>
          {groupedDocuments.map((targetMonthDocuments, idx) => (
            <>
              <Title key={`title=${idx}`} order={3} mt={'lg'} mb="sm">
                {format(new Date(targetMonthDocuments[0].datetime), 'MMM yyyy')}
              </Title>
              <SimpleGrid key={`grid=${idx}`} cols={{ base: 1, sm: 2, md: 4 }}
                          spacing={{ base: 'ms', md: 'md', lg: 'lg' }}>
                {targetMonthDocuments.map((document, idx) => (
                  <AccountDocumentLine onClick={setLightboxSrc} key={idx} {...document} />
                ))}
              </SimpleGrid>
            </>
          ))}
        </>
      ) : (
        <Table verticalSpacing="xs" highlightOnHover>
          <Table.Thead>
            <TableRow>
              <Table.Th>Filename</Table.Th>
              <Table.Th style={{}}>Linked Directive</Table.Th>
              <Table.Th>Created Date</Table.Th>
              <Table.Th>Operation</Table.Th>
            </TableRow>
          </Table.Thead>
          <tbody>
          {documents.map((document, idx) => (
            <TableRow>
              <TableCell onClick={isDocumentAnImage(document.path) ? () => setLightboxSrc(document.path) : undefined}>
                <div>{document.filename}</div>
              </TableCell>
              <TableCell>
                {document.account &&
                  <TextBadge onClick={() => navigate(`/accounts/${document.account}`)}>{document.account}</TextBadge>}
                {document.trx_id && <TextBadge key={idx}>{document.trx_id}</TextBadge>}
              </TableCell>
              <TableCell>{format(new Date(document.datetime), 'yyyy-MM-dd HH:mm:ss')}</TableCell>
              <TableCell></TableCell>
            </TableRow>
          ))}
          </tbody>
        </Table>
      )}
    </Container>
  );
}
