import { Badge, Container, Group, SimpleGrid, Stack, Table, Tabs, Text, Title } from '@mantine/core';
import { IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons-react';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '../global.ts';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import LoadingComponent from '../components/basic/LoadingComponent';
import PayeeNarration from '../components/basic/PayeeNarration';
import { AccountBalanceHistory, AccountInfo, AccountJournalItem, Document } from '../rest-model';
import DocumentPreview from '../components/journalPreview/DocumentPreview';
import { useDocumentTitle } from '@mantine/hooks';
import { createStyles } from '@mantine/emotion';
import { AccountBalanceHistoryGraph } from '../components/AccountBalanceHistoryGraph';
import { useState } from 'react';
import { ImageLightBox } from '../components/ImageLightBox';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';

const useStyles = createStyles((theme, _) => ({
  calculatedAmount: {
    fontSize: `calc(${theme.fontSizes.xl} * 1.1)`,
    fontWeight: 500,
  },
  detailAmount: {
    fontSize: theme.fontSizes.lg,
  },
}));

function SingleAccount() {
  let { accountName } = useParams();
  const { classes } = useStyles();

  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);

  const { data: account, error } = useSWR<AccountInfo>(`/api/accounts/${accountName}`, fetcher);
  const {
    data: account_balance_data,
    error: account_balance_error,
  } = useSWR<AccountBalanceHistory>(`/api/accounts/${accountName}/balances`, fetcher);

  const ledgerTitle = useAtomValue(titleAtom);
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
      {account_balance_error ? (
        <div>fail to fetch account balance history</div>
      ) : (
        account_balance_data && <AccountBalanceHistoryGraph data={account_balance_data} />
      )}

      <Tabs keepMounted={false} variant="outline" defaultValue="journals" mt="lg">
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
              <TableRow>
                <Table.Th>Date</Table.Th>
                <Table.Th>Payee & Narration</Table.Th>
                <Table.Th style={{ textAlign: 'right' }}>Change Amount</Table.Th>
                <Table.Th style={{ textAlign: 'right' }}>After Change Amount</Table.Th>
              </TableRow>
            </Table.Thead>
            <tbody>
            <LoadingComponent
              url={`/api/accounts/${accountName}/journals`}
              skeleton={<div>loading</div>}
              render={(data: AccountJournalItem[]) => (
                <>
                  {(data ?? []).map((item) => (
                    <TableRow>
                      <TableCell>{format(new Date(item.datetime), 'yyyy-MM-dd HH:mm:ss')}</TableCell>
                      <TableCell>
                        <PayeeNarration payee={item.payee} narration={item.narration} />
                      </TableCell>
                      <TableCell style={{ textAlign: 'right' }}>
                        <Amount amount={item.inferred_unit_number} currency={item.inferred_unit_commodity} />
                      </TableCell>
                      <TableCell style={{ textAlign: 'right' }}>
                        <Amount amount={item.account_after_number} currency={item.account_after_commodity} />
                      </TableCell>
                    </TableRow>
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
                <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />
                <SimpleGrid cols={{ base: 4, md: 3, sm: 2, xs: 1 }} spacing={{ base: 'sm', md: 'md', sm: 'sm' }}>
                  <AccountDocumentUpload url={`/api/accounts/${accountName}/documents`} />
                  {data.map((document, idx) => (
                    <DocumentPreview onClick={(path) => setLightboxSrc(path)} key={idx} uri={document.path}
                                     filename={document.path} />
                  ))}
                </SimpleGrid>
              </>
            )}
          ></LoadingComponent>
        </Tabs.Panel>

        <Tabs.Panel value="settings" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <Table.Thead>
              <TableRow>
                <Table.Th>Currency</Table.Th>
                <Table.Th>Current Balance</Table.Th>
                <Table.Th>Latest Balance Time</Table.Th>
                <Table.Th>Pad Account</Table.Th>
                <Table.Th>Destination</Table.Th>
              </TableRow>
            </Table.Thead>
            <tbody>
            {Object.entries(account?.amount.detail ?? {}).map(([commodity, amount]) => (
              <AccountBalanceCheckLine key={commodity} currentAmount={amount} commodity={commodity}
                                       accountName={account.name} />
            ))}
            </tbody>
          </Table>
        </Tabs.Panel>
      </Tabs>
    </Container>
  );
}

export default SingleAccount;
