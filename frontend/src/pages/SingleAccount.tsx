import { Container, Group, Table, Tabs, Title } from '@mantine/core';
import { IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import LoadingComponent from '../components/basic/LoadingComponent';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { AccountJournalItem, Document, LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchAccounts, getAccountByName } from '../states/account';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import { useEffect } from 'react';
import PayeeNarration from '../components/basic/PayeeNarration';
import Amount from '../components/Amount';

function SingleAccount() {
  let { accountName } = useParams();
  const dispatch = useAppDispatch();

  const account = useAppSelector(getAccountByName(accountName!));
  const accountStatus = useAppSelector((state) => state.accounts.status);

  useEffect(() => {
    if (accountStatus === LoadingState.NotReady) {
      dispatch(fetchAccounts());
    }
  }, [dispatch, accountStatus]);

  return (
    <Container fluid>
      <Title order={2}>{accountName}</Title>
      <Group>{/* <Badge variant="outline">{data?.account.status}</Badge> */}</Group>
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
                <th style={{ textAlign: 'right' }}>Change Amount</th>
                <th style={{ textAlign: 'right' }}>After Change Amount</th>
              </tr>
            </thead>
            <tbody>
              <LoadingComponent
                url={`/api/accounts/${accountName}/journals`}
                skeleton={<div>loading</div>}
                render={(data: AccountJournalItem[]) => (
                  <>
                    {data.map((item) => (
                      <tr>
                        <td>{format(new Date(item.datetime), 'yyyy-MM-dd hh:mm:ss')}</td>
                        <td>
                          <PayeeNarration payee={item.payee} narration={item.narration} />
                        </td>
                        <td style={{ textAlign: 'right' }}>
                          <Amount amount={item.inferred_unit_number} currency={item.inferred_unit_commodity} />
                        </td>
                        <td style={{ textAlign: 'right' }}>
                          <Amount amount={item.account_after_number} currency={item.account_after_commodity} />
                        </td>
                      </tr>
                    ))}
                  </>
                )}
              />
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
            )}
          ></LoadingComponent>
        </Tabs.Panel>

        <Tabs.Panel value="settings" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Currency</th>
                <th>Current Balance</th>
                <th>Latest Balance Time</th>
                <th>Pad Account</th>
                <th>Distanation</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(account?.amount.detail ?? []).map(([commodity, amount], idx) => (
                <AccountBalanceCheckLine currentAmount={amount} commodity={commodity} accountName={account.name} />
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>
      </Tabs>
    </Container>
  );
}

export default SingleAccount;
