import { Container, Group, Table, Tabs, Title } from '@mantine/core';
import { IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import LoadingComponent from '../components/basic/LoadingComponent';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { AccountJournalItem, Document } from '../rest-model';

function SingleAccount() {
  let { accountName } = useParams();

  return (
    <Container fluid>
      <Title order={2}>{accountName}</Title>
      <Group>
        {/* <Badge variant="outline">{data?.account.status}</Badge> */}
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
                <th>Payee & Narration</th>
                <th>Change Amount</th>
                <th>After Change Amount</th>
              </tr>
            </thead>
            <tbody>
              <LoadingComponent
                url={`/api/accounts/${accountName}/journals`}
                skeleton={<div>loading</div>}
                render={(data: AccountJournalItem[]) => (
                  <>
                    {data.map(item => (
                      <tr>
                        <td>{format(new Date(item.datetime), 'yyyy-MM-dd hh:mm:ss')}</td>
                        <td>{item.payee} {item.narration}</td>
                        <td>{item.inferred_unit_number} {item.inferred_unit_commodity}</td>
                        <td>{item.account_after_number} {item.account_after_commodity}</td>
                      </tr>
                    ))}
                  </>
                )} />
            </tbody>
          </Table>
        </Tabs.Panel>

        <Tabs.Panel value="documents" pt="xs">
          <LoadingComponent
            url={`/api/accounts/${accountName}/documents`}
            skeleton={<div>loading</div>}
            render={(data: Document[]) => (
              <>
                <AccountDocumentUpload url={`/api/accounts/${accountName}/documents`} />
                {data.map((document, idx) => (
                  <AccountDocumentLine key={idx} {...document} />
                ))}
              </>
            )}></LoadingComponent>
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
              {/* {data?.account.currencies.map((it, idx) => (
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
              ))} */}
            </tbody>
          </Table>
        </Tabs.Panel>
      </Tabs>
    </Container>
  );
}

export default SingleAccount;
