import { Badge, Container, createStyles, Group, px, SimpleGrid, Stack, Table, Tabs, Text, Title } from '@mantine/core';
import { IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '..';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import LoadingComponent from '../components/basic/LoadingComponent';
import PayeeNarration from '../components/basic/PayeeNarration';
import { AccountInfo, AccountJournalItem, Document } from '../rest-model';
import DocumentPreview from '../components/journalPreview/DocumentPreview';
import { useAppSelector } from '../states';
import { useDocumentTitle } from '@mantine/hooks';

const useStyles = createStyles((theme) => ({
  calculatedAmount: {
    fontSize: px(theme.fontSizes.xl) * 1.1,
    fontWeight: 500,
  },
  detailAmount: {
    fontSize: px(theme.fontSizes.lg),
  },
}));

function SingleAccount() {
  let { accountName } = useParams();
  const { classes } = useStyles();

  const { data: account, error } = useSWR<AccountInfo>(`/api/accounts/${accountName}`, fetcher);

  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`${accountName} | Accounts - ${ledgerTitle}`);

  if (error) return <div>failed to load</div>;
  if (!account) return <div>{error}</div>;
  return (
    <Container fluid>
      <Group position="apart" py="md" px="sm" align="baseline">
        <Stack>
          <Title order={2}>{account.alias ?? account.name}</Title>
          <Group>
            <Badge>{account.status}</Badge>
            {!!account.alias && <Title order={4}>{account.name}</Title>}
          </Group>
        </Stack>
        <Stack align="end" spacing="xs">
          <Group className={classes.calculatedAmount}>
            {Object.keys(account.amount.detail).length > 1 && <Text>≈</Text>}
            <Amount amount={account.amount.calculated.number} currency={account.amount.calculated.currency}></Amount>
          </Group>
          {Object.keys(account.amount.detail).length > 1 && (
            <>
              {Object.entries(account.amount.detail).map(([key, value]) => (
                <Amount key={key} className={classes.detailAmount} amount={value} currency={key}></Amount>
              ))}
            </>
          )}
        </Stack>
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
                        <td>{format(new Date(item.datetime), 'yyyy-MM-dd HH:mm:ss')}</td>
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
                <SimpleGrid
                  cols={4}
                  spacing="sm"
                  breakpoints={[
                    { maxWidth: 'md', cols: 3, spacing: 'md' },
                    { maxWidth: 'sm', cols: 2, spacing: 'sm' },
                    { maxWidth: 'xs', cols: 1, spacing: 'sm' },
                  ]}
                >
                  <AccountDocumentUpload url={`/api/accounts/${accountName}/documents`} />
                  {data.map((document, idx) => (
                    <DocumentPreview key={idx} uri={document.path} filename={document.path} />
                  ))}
                </SimpleGrid>
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
