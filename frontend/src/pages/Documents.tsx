import { Button, Container, Group, SimpleGrid, Table, Title } from '@mantine/core';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { openContextModal } from '@mantine/modals';
import { IconLayout2, IconListDetails } from '@tabler/icons';
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
  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`Documents - ${ledgerTitle}`);

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

  const groupedDocuments = reverse(
    sortBy(
      groupBy(documents, (document) => format(new Date(document.datetime), 'yyyy-MM')),
      (it) => it[0].datetime,
    ),
  );
  console.log(groupedDocuments);
  return (
    <Container fluid>
      <Group position="apart">
        <Heading title={`${documents.length} Documents`}></Heading>
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
          {groupedDocuments.map((targetMonthDocuments, idx) => (
            <>
              <Title key={`title=${idx}`} order={3} mt={'lg'} mb="sm">
                {format(new Date(targetMonthDocuments[0].datetime), 'MMM yyyy')}
              </Title>
              <SimpleGrid
                key={`grid=${idx}`}
                cols={4}
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
                </td>
                <td>
                  {document.account && <TextBadge onClick={() => navigate(`/accounts/${document.account}`)}>{document.account}</TextBadge>}
                  {document.trx_id && <TextBadge key={idx}>{document.trx_id}</TextBadge>}
                </td>
                <td>{format(new Date(document.datetime), 'yyyy-MM-dd HH:mm:ss')}</td>
                <td></td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
    </Container>
  );
}
