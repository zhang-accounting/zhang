import { useQuery } from '@apollo/client';
import { Badge, Container, Group, Table, Tabs, Title } from '@mantine/core';
import { IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons';
import { format } from 'date-fns';
import { maxBy } from 'lodash';
import { useParams } from 'react-router';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import AccountDocumentLine, { DocumentRenderItem } from '../components/documentLines/AccountDocumentLine';
import JournalLine from '../components/JournalLine';
import { CommodityBalanceTime } from '../gql/accountList';
import { SingleAccountJournalQuery, SINGLE_ACCONT_JOURNAL } from '../gql/singleAccount';

function SingleAccount() {
  let { accountName } = useParams();

  const getLatestBalanceTime = (commodity: string, times: CommodityBalanceTime[]) => {
    const latestTime = maxBy(
      times.filter((time) => time.commodity === commodity),
      (time) => time.date,
    );
    if (latestTime) {
      return format(new Date(latestTime.date * 1000), 'yyyy-MM-dd');
    } else {
      return 'N/A';
    }
  };

  const { loading, error, data } = useQuery<SingleAccountJournalQuery>(SINGLE_ACCONT_JOURNAL, {
    variables: {
      name: accountName,
    },
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  const documents: { [filename: string]: DocumentRenderItem } = {};

  for (let document of data!.account.documents) {
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
      <Title order={2}>{accountName}</Title>
      <Group>
        <Badge variant="outline">{data?.account.status}</Badge>
      </Group>
      <Tabs defaultValue="journals" mt="lg">
        <Tabs.List>
          <Tabs.Tab value="journals" icon={<IconPhoto size={14} />}>
            Journals
          </Tabs.Tab>
          <Tabs.Tab value="documents" icon={<IconMessageCircle size={14} />}>
            Documents
          </Tabs.Tab>
          <Tabs.Tab value="settings" icon={<IconSettings size={14} />}>
            Settings
          </Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="journals" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Date</th>
                <th style={{}}>Payee & Narration</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {data?.account.journals.map((journal, idx) => (
                <JournalLine key={idx} data={journal} />
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>

        <Tabs.Panel value="documents" pt="xs">
          <AccountDocumentUpload accountName={data!.account.name} />
          {Object.values(documents).map((document, idx) => (
            <AccountDocumentLine key={idx} {...document} />
          ))}
        </Tabs.Panel>

        <Tabs.Panel value="settings" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Currency</th>
                <th>Current Balance</th>
                <th>Latest Balance Time</th>
                <th>Distanation</th>
              </tr>
            </thead>
            <tbody>
              {data?.account.currencies.map((it, idx) => (
                <tr key={idx}>
                  <td>{it.name}</td>
                  <td>
                    <Amount amount={data.account.snapshot.detail.find((cur) => cur.currency === it.name)?.number || '0.00'} currency={it.name} />
                  </td>
                  <td>{getLatestBalanceTime(it.name, data!.account.latestBalanceTimes)}</td>
                  <td>
                    <AccountBalanceCheckLine currency={it.name} accountName={data.account.name} />
                  </td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>
      </Tabs>
    </Container>
  );
}

export default SingleAccount;
