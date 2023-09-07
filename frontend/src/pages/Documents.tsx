import { Badge, Button, Container, Group, SimpleGrid, Table, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { openContextModal } from '@mantine/modals';
import { IconLayout2, IconListDetails } from '@tabler/icons';
import { format } from 'date-fns';
import { groupBy } from 'lodash-es';
import useSWR from 'swr';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { fetcher } from '../index';
import { Document } from '../rest-model';

export default function Documents() {
  const [layout, setLayout] = useLocalStorage({ key: `document-list-layout`, defaultValue: 'Grid' });

  const { data: documents, error } = useSWR<Document[]>('/api/documents', fetcher);

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

  const groupedDocuments = groupBy(documents, (document) => format(new Date(document.datetime), 'yyyy-MM'));
  return (
    <Container fluid>
      <Group position="apart">
        <Title order={2}>{documents.length} Documents</Title>
        <Button.Group>
          <Button disabled={layout === 'Grid'} leftIcon={<IconLayout2 size={14} />} variant="default" onClick={() => setLayout('Grid')}>
            Grid
          </Button>
          <Button disabled={layout === 'Table'} leftIcon={<IconListDetails size={14} />} variant="default" onClick={() => setLayout('Table')}>
            Table
          </Button>
        </Button.Group>
      </Group>

      {layout === 'Grid' ? (
        <>
          {Object.values(groupedDocuments).map((targetMonthDocuments, idx) => (
            <>
              <Title key={`title=${idx}`} order={3} mt={'lg'} mb="sm">
                {format(new Date(targetMonthDocuments[0].datetime), 'MMM yyyy')}
              </Title>
              <SimpleGrid
                key={`grid=${idx}`}
                cols={6}
                spacing="lg"
                breakpoints={[
                  { maxWidth: 'lg', cols: 4, spacing: 'md' },
                  { maxWidth: 'sm', cols: 2, spacing: 'sm' },
                  { maxWidth: 'xs', cols: 1, spacing: 'sm' },
                ]}
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
          <thead>
            <tr>
              <th>Filename</th>
              <th style={{}}>Linked Directive</th>
              <th>Created Date</th>
              <th>Operation</th>
            </tr>
          </thead>
          <tbody>
            {documents.map((document, idx) => (
              <tr>
                <td onClick={() => openDocumentPreviewModal(document.filename, document.path)}>
                  <div>{document.filename}</div>
                  <Badge color="dark">{document.filename.split('.').pop()}</Badge>
                </td>
                <td>
                  {document.account && <Badge variant="dot">{document.account}</Badge>}
                  {document.trx_id && (
                    <Badge key={idx} variant="dot" color="violet">
                      {document.trx_id}
                    </Badge>
                  )}
                </td>
                <td>{format(new Date(document.datetime), 'yyyy-MM-dd hh:mm:ss')}</td>
                <td></td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
    </Container>
  );
}
