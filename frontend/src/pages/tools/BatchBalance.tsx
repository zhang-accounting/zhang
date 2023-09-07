import { Button, Chip, Container, Group, Select, Table, TextInput, Title } from '@mantine/core';
import { useListState, useLocalStorage } from '@mantine/hooks';
import { createSelector } from '@reduxjs/toolkit';
import { cloneDeep, sortBy } from 'lodash-es';
import { useEffect } from 'react';
import Amount from '../../components/Amount';
import { Account, LoadingState } from '../../rest-model';
import { useAppDispatch, useAppSelector } from '../../states';
import { accountsSlice, fetchAccounts, getAccountSelectItems } from '../../states/account';
import BigNumber from 'bignumber.js';
import { showNotification } from '@mantine/notifications';
import { axiosInstance } from '../..';

interface BalanceLineItem {
  commodity: string;
  currentAmount: string;
  accountName: string;

  balanceAmount: string;
  pad?: string;
  error: boolean;
}

const getFilteredItems = createSelector([(states) => states.accounts], (accounts) => {
  const items = accounts.data.flatMap((account: Account) =>
    Object.entries(account.amount.detail).map(([commodity, value]) => ({
      commodity: commodity,
      currentAmount: value,
      accountName: account.name,
      balanceAmount: '',
      pad: undefined,
      error: false,
    })),
  );
  return sortBy(cloneDeep(items), (item) => item.accountName);
});

export default function BatchBalance() {
  const dispatch = useAppDispatch();
  const accountStatus = useAppSelector((state) => state.accounts.status);
  const stateItems = useAppSelector(getFilteredItems);
  const accountSelectItems = [...useAppSelector(getAccountSelectItems())];
  const [accounts, accountsHandler] = useListState<BalanceLineItem>(stateItems);

  const [maskCurrentAmount, setMaskCurrentAmount] = useLocalStorage({ key: 'tool/maskCurrentAmount', defaultValue: false });
  const [reflectOnUnbalancedAmount, setReflectOnUnbalancedAmount] = useLocalStorage({ key: 'tool/reflectOnUnbalancedAmount', defaultValue: true });

  useEffect(() => {
    if (accountStatus === LoadingState.NotReady) {
      dispatch(fetchAccounts());
    }
  }, [accountStatus, dispatch]);
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
      dispatch(accountsSlice.actions.clear());
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
        <thead>
          <tr>
            <th>Account</th>
            <th>Commodity</th>
            <th>Current Balance</th>
            <th>Pad Account</th>
            <th>Destination</th>
          </tr>
        </thead>
        <tbody>
          {accounts.map((account, idx) => (
            <tr key={`${account.accountName}-${account.commodity}`}>
              <td>{account.accountName}</td>
              <td>{account.commodity}</td>
              <td>
                <Amount mask={maskCurrentAmount} amount={account.currentAmount} currency={account.commodity}></Amount>
              </td>
              <td>
                <Select
                  searchable
                  clearable
                  placeholder="Pad to"
                  data={accountSelectItems}
                  value={account.pad}
                  onChange={(e) => {
                    updateBalanceLineItem(idx, e ?? undefined, account.balanceAmount);
                  }}
                />
              </td>
              <td>
                <TextInput
                  error={account.error}
                  value={account.balanceAmount}
                  onChange={(e) => {
                    updateBalanceLineItem(idx, account.pad ?? undefined, e.target.value);
                  }}
                ></TextInput>
              </td>
            </tr>
          ))}
        </tbody>
      </Table>
      <Button disabled={accounts.filter((account) => account.balanceAmount.trim() !== '').length === 0} onClick={onSave}>
        Submit
      </Button>
    </Container>
  );
}
