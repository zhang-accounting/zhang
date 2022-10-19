import { useQuery } from '@apollo/client';
import { Container, Grid, Title } from '@mantine/core';
import AccountDocumentLine, { DocumentRenderItem } from '../components/documentLines/AccountDocumentLine';
import { DocumentListQuery, DOCUMENT_LIST } from '../gql/documentList';

export default function Documents() {
  const { loading, error, data } = useQuery<DocumentListQuery>(DOCUMENT_LIST);

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
      <Title order={2}>{Object.keys(documents).length} Documents</Title>
      <Grid gutter="xs" mt="lg">
        {Object.values(documents).map((document, idx) => (
          <Grid.Col key={idx} span={3}>
            <AccountDocumentLine {...document} />
          </Grid.Col>
        ))}
      </Grid>
    </Container>
  );
}
