import { Container, Group, SegmentedControl, SimpleGrid, Table, Title } from '@mantine/core';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { openContextModal } from '@mantine/modals';
import { format } from 'date-fns';
import useSWR from 'swr';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { fetcher } from '../index';
import { Document } from '../rest-model';
import { Heading } from '../components/basic/Heading';
import { groupBy, reverse, sortBy } from 'lodash-es';
import { TextBadge } from '../components/basic/TextBadge';
import { useNavigate } from 'react-router';
import { useAppSelector } from '../states';

export default function Documents() {
  let navigate = useNavigate();
  const [layout, setLayout] = useLocalStorage({ key: `document-list-layout`, defaultValue: 'Grid' });
  const { data: documents, error } = useSWR<Document[]>('/api/documents', fetcher);

  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`Documents - ${ledgerTitle}`);

  if (error) return <div>failed to load</div>;
  if (!documents) return <div>loading...</div>;
  const openDocumentPreviewModal = (filename: string, path: string) => {
    openContextModal({
      modal: 'documentPreviewModal',
      title: filename,
      size: 'lg',
      centered: true,
      innerProps: {
        filename: filename,
        path: path,
      },
    });
  };

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

      {layout === 'Grid' ? (
        <>
          {groupedDocuments.map((targetMonthDocuments, idx) => (
            <>
              <Title key={`title=${idx}`} order={3} mt={'lg'} mb="sm">
                {format(new Date(targetMonthDocuments[0].datetime), 'MMM yyyy')}
              </Title>
              <SimpleGrid
                key={`grid=${idx}`}
                cols={{ base: 4, sm: 2, xs: 1 }}
                spacing={{ base: 'lg', lg: 'md', sm: 'sm' }}
              >
                {targetMonthDocuments.map((document, idx) => (
                  <AccountDocumentLine key={idx} {...document} />
                ))}
              </SimpleGrid>
            </>
          ))}
        </>
      ) : (
        <Table verticalSpacing="xs" highlightOnHover>
          <Table.Thead>
          <Table.Tr>
            <Table.Th>Filename</Table.Th>
            <Table.Th style={{}}>Linked Directive</Table.Th>
            <Table.Th>Created Date</Table.Th>
            <Table.Th>Operation</Table.Th>
          </Table.Tr>
          </Table.Thead>
          <tbody>
          {documents.map((document, idx) => (
            <Table.Tr>
              <Table.Td onClick={() => openDocumentPreviewModal(document.filename, document.path)}>
                <div>{document.filename}</div>
              </Table.Td>
              <Table.Td>
                {document.account &&
                  <TextBadge onClick={() => navigate(`/accounts/${document.account}`)}>{document.account}</TextBadge>}
                {document.trx_id && <TextBadge key={idx}>{document.trx_id}</TextBadge>}
              </Table.Td>
              <Table.Td>{format(new Date(document.datetime), 'yyyy-MM-dd HH:mm:ss')}</Table.Td>
              <Table.Td></Table.Td>
            </Table.Tr>
          ))}
          </tbody>
        </Table>
      )}
    </Container>
  );
}
