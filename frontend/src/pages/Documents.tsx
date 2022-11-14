import { useQuery } from '@apollo/client';
import { Badge, Button, Container, Grid, Group, Table, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { IconLayout2, IconListDetails } from '@tabler/icons';
import AccountDocumentLine, { DocumentRenderItem } from '../components/documentLines/AccountDocumentLine';
import { DocumentListQuery, DOCUMENT_LIST } from '../gql/documentList';

export default function Documents() {
  const { loading, error, data } = useQuery<DocumentListQuery>(DOCUMENT_LIST);
  const [layout, setLayout] = useLocalStorage({ key: `document-list-layout`, defaultValue: "Grid" })

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;
  const documents: { [filename: string]: DocumentRenderItem } = {};

  for (let document of data!.documents) {
    let filename = document.filename;
    if (!documents.hasOwnProperty(filename)) {
      documents[filename] = {
        filename: filename,
        accounts: [],
        transactions: [],
      } as DocumentRenderItem;
    }

    switch (document.__typename) {
      case 'AccountDocumentDto':
        documents[filename].accounts.push(document.account);
        break;
      case 'TransactionDocumentDto':
        documents[filename].transactions.push(document.transaction);
        break;
      default:
    }
  }
  return (
    <Container fluid>
      <Group position='apart'>
        <Title order={2}>{Object.keys(documents).length} Documents</Title>
        <Button.Group>
          <Button disabled={layout === "Grid"} leftIcon={<IconLayout2 size={14} />} variant="default" onClick={() => setLayout("Grid")}>Grid</Button>
          <Button disabled={layout === "Table"} leftIcon={<IconListDetails size={14} />} variant="default" onClick={() => setLayout("Table")}>Table</Button>
        </Button.Group>
      </Group>

      {
        layout === "Grid" ?

          <Grid gutter="xs" mt="lg">
            {Object.values(documents).map((document, idx) => (
              <Grid.Col key={idx} span={3}>
                <AccountDocumentLine {...document} />
              </Grid.Col>
            ))}
          </Grid> : (
            <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>File Type</th>
                <th>Filename</th>
                <th style={{}}>Linked Directive</th>
                <th>Created Date</th>
                <th>Operation</th>
              </tr>
            </thead>
            <tbody>
            {Object.values(documents).map((document, idx) => (
              <tr>
                <td><Badge color="dark">{document.filename.split(".").pop()}</Badge></td>
                <td>{document.filename.split("/").pop()}</td>
                <td>{(document.accounts || []).map(account => <Badge key={account?.name} variant="dot">{account?.name}</Badge>)}
                {(document.transactions || []).map((trx, idx) => <Badge key={idx} variant="dot" color="violet">{trx?.payee}</Badge>)}
                </td>
                <td></td>
                <td></td>
              </tr>
              
            ))}
            </tbody>
          </Table>
          )
      }

    </Container>
  );
}
