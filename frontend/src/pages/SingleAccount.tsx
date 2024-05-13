import { Badge, Container, Group, px, SimpleGrid, Stack, Table, Tabs, Text, Title } from '@mantine/core';
import { IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons-react';
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
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  calculatedAmount: {
    fontSize: `calc(${theme.fontSizes.xl} * 1.1)`,
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

  console.log('account data', account);
  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`${accountName} | Accounts - ${ledgerTitle}`);

  if (error) return <div>failed to load</div>;
  if (!account) return <div>{error}</div>;
  return (
    <Container fluid>
      <Group justify="space-between" py="md" px="sm" align="baseline">
        <Stack>
          <Title order={2}>{account.alias ?? account.name}</Title>
          <Group>
            <Badge>{account.status}</Badge>
            {!!account.alias && <Title order={4}>{account.name}</Title>}
          </Group>
        </Stack>
        <Stack align="end" gap="xs">
          <Group className={classes.calculatedAmount}>
            {Object.keys(account.amount.detail).length > 1 && <Text>≈</Text>}
            <Amount amount={account.amount.calculated.number} currency={account.amount.calculated.currency}></Amount>
          </Group>
          {Object.keys(account.amount.detail).length > 1 && (
            <>
              {Object.entries(account.amount.detail ?? {}).map(([key, value]) => (
                <Amount key={key} className={classes.detailAmount} amount={value} currency={key}></Amount>
              ))}
            </>
          )}
        </Stack>
      </Group>
      <Tabs defaultValue="journals" mt="lg">
        <Tabs.List>
          <Tabs.Tab value="journals" leftSection={<IconPhoto size={14} />}>
            Journals
          </Tabs.Tab>
          <Tabs.Tab value="documents" leftSection={<IconMessageCircle size={14} />}>
            Documents
          </Tabs.Tab>
          <Tabs.Tab value="settings" leftSection={<IconSettings size={14} />}>
            Settings
          </Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="journals" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Date</Table.Th>
                <Table.Th>Payee & Narration</Table.Th>
                <Table.Th style={{ textAlign: 'right' }}>Change Amount</Table.Th>
                <Table.Th style={{ textAlign: 'right' }}>After Change Amount</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <tbody>
              <LoadingComponent
                url={`/api/accounts/${accountName}/journals`}
                skeleton={<div>loading</div>}
                render={(data: AccountJournalItem[]) => (
                  <>
                    {(data ?? []).map((item) => (
                      <Table.Tr>
                        <Table.Td>{format(new Date(item.datetime), 'yyyy-MM-dd HH:mm:ss')}</Table.Td>
                        <Table.Td>
                          <PayeeNarration payee={item.payee} narration={item.narration} />
                        </Table.Td>
                        <Table.Td style={{ textAlign: 'right' }}>
                          <Amount amount={item.inferred_unit_number} currency={item.inferred_unit_commodity} />
                        </Table.Td>
                        <Table.Td style={{ textAlign: 'right' }}>
                          <Amount amount={item.account_after_number} currency={item.account_after_commodity} />
                        </Table.Td>
                      </Table.Tr>
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
                <SimpleGrid cols={{ base: 4, md: 3, sm: 2, xs: 1 }} spacing={{ base: 'sm', md: 'md', sm: 'sm' }}>
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
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Currency</Table.Th>
                <Table.Th>Current Balance</Table.Th>
                <Table.Th>Latest Balance Time</Table.Th>
                <Table.Th>Pad Account</Table.Th>
                <Table.Th>Distanation</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <tbody>
              {Object.entries(account?.amount.detail ?? {}).map(([commodity, amount], idx) => (
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
