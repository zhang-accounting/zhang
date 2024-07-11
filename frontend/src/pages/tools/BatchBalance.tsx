import { Button, Chip, Container, Group, Select, Table, TextInput, Title } from '@mantine/core';
import { useListState, useLocalStorage } from '@mantine/hooks';
import { useEffect, useMemo } from 'react';
import Amount from '../../components/Amount';
import { loadable_unwrap } from '../../states';
import { accountAtom, accountFetcher, accountSelectItemsAtom } from '../../states/account';
import BigNumber from 'bignumber.js';
import { showNotification } from '@mantine/notifications';
import { axiosInstance } from '../..';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { selectAtom } from 'jotai/utils';

interface BalanceLineItem {
  commodity: string;
  currentAmount: string;
  accountName: string;

  balanceAmount: string;
  pad?: string;
  error: boolean;
}

export default function BatchBalance() {
  const stateItems = useAtomValue(
    useMemo(
      () =>
        selectAtom(accountAtom, (val) =>
          loadable_unwrap(val, [], (data) => {
            return data.flatMap((account) =>
              Object.entries(account.amount.detail).map(([commodity, value]) => ({
                commodity: commodity,
                currentAmount: value,
                accountName: account.name,
                balanceAmount: '',
                pad: undefined,
                error: false,
              })),
            );
          }),
        ),
      [],
    ),
  );

  const [accounts, accountsHandler] = useListState<BalanceLineItem>(stateItems);
  const accountItems = useAtomValue(accountSelectItemsAtom);
  const refreshAccounts = useSetAtom(accountFetcher);
  const [maskCurrentAmount, setMaskCurrentAmount] = useLocalStorage({
    key: 'tool/maskCurrentAmount',
    defaultValue: false,
  });
  const [reflectOnUnbalancedAmount, setReflectOnUnbalancedAmount] = useLocalStorage({
    key: 'tool/reflectOnUnbalancedAmount',
    defaultValue: true,
  });

  useEffect(() => {
    accountsHandler.setState(stateItems);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stateItems]);

  const updateBalanceLineItem = (idx: number, padAccount: string | undefined, balanceAmount: string) => {
    const targetAccount = accounts[idx];
    accountsHandler.setItemProp(idx, 'pad', padAccount);
    accountsHandler.setItemProp(idx, 'balanceAmount', balanceAmount);

    if (reflectOnUnbalancedAmount) {
      if (padAccount === undefined) {
        if (balanceAmount.trim() !== '') {
          const isBlanced = new BigNumber(targetAccount.currentAmount).eq(new BigNumber(balanceAmount));
          accountsHandler.setItemProp(idx, 'error', !isBlanced);
          return;
        }
      }
    }
    accountsHandler.setItemProp(idx, 'error', false);
  };

  const onSave = async () => {
    const accountsToBlance = accounts
      .filter((account) => account.balanceAmount.trim() !== '')
      .map((account) => ({
        type: account.pad ? 'Pad' : 'Check',
        account_name: account.accountName,
        amount: {
          number: account.balanceAmount,
          commodity: account.commodity,
        },
        pad: account.pad,
      }));
    showNotification({
      title: `Start balance ${accountsToBlance.length} Accounts`,
      message: '',
    });
    try {
      await axiosInstance.post('/api/accounts/batch-balances', accountsToBlance);
      showNotification({
        title: 'Balance account successfully',
        message: 'waiting page to refetch latest data',
      });
      refreshAccounts();
    } catch (e: any) {
      showNotification({
        title: 'Fail to Balance Account',
        color: 'red',
        message: e?.response?.data ?? '',
        autoClose: false,
      });
    }
  };
  return (
    <Container fluid>
      <Title order={2}>Batch Balance</Title>
      <Group my="lg">
        <Chip checked={maskCurrentAmount} onChange={() => setMaskCurrentAmount(!maskCurrentAmount)}>
          Mask Current Amount
        </Chip>
        <Chip checked={reflectOnUnbalancedAmount} onChange={() => setReflectOnUnbalancedAmount(!reflectOnUnbalancedAmount)}>
          Reflect on unbalanced amount
        </Chip>
      </Group>
      <Table verticalSpacing="xs" highlightOnHover>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Account</Table.Th>
            <Table.Th>Commodity</Table.Th>
            <Table.Th>Current Balance</Table.Th>
            <Table.Th>Pad Account</Table.Th>
            <Table.Th>Destination</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <tbody>
          {accounts.map((account, idx) => (
            <Table.Tr key={`${account.accountName}-${account.commodity}`}>
              <Table.Td>{account.accountName}</Table.Td>
              <Table.Td>{account.commodity}</Table.Td>
              <Table.Td>
                <Amount mask={maskCurrentAmount} amount={account.currentAmount} currency={account.commodity}></Amount>
              </Table.Td>
              <Table.Td>
                <Select
                  searchable
                  clearable
                  placeholder="Pad to"
                  data={accountItems}
                  value={account.pad}
                  onChange={(e) => {
                    updateBalanceLineItem(idx, e ?? undefined, account.balanceAmount);
                  }}
                />
              </Table.Td>
              <Table.Td>
                <TextInput
                  error={account.error}
                  value={account.balanceAmount}
                  onChange={(e) => {
                    updateBalanceLineItem(idx, account.pad ?? undefined, e.target.value);
                  }}
                ></TextInput>
              </Table.Td>
            </Table.Tr>
          ))}
        </tbody>
      </Table>
      <Button disabled={accounts.filter((account) => account.balanceAmount.trim() !== '').length === 0} onClick={onSave}>
        Submit
      </Button>
    </Container>
  );
}
