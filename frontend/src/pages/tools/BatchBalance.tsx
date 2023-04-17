import { Button, Chip, Container, Group, Select, Table, TextInput, Title } from '@mantine/core';
import { useListState, useLocalStorage } from '@mantine/hooks';
import { createSelector } from '@reduxjs/toolkit';
import { cloneDeep, sortBy } from 'lodash';
import { useEffect } from 'react';
import Amount from '../../components/Amount';
import { Account, LoadingState } from '../../rest-model';
import { useAppDispatch, useAppSelector } from "../../states";
import { fetchAccounts, getAccountSelectItems } from "../../states/account";


interface BalanceLineItem {
  commodity: string,
  currentAmount: string,
  accountName: string,

  balanceAmount: string,
  pad?: string
}

const getFilteredItems = createSelector(
  [(states) => states.accounts],
  accounts => {
    const items = accounts.data.flatMap((account: Account) => Object.entries(account.commodities).map(([commodity, value]) => ({
      commodity: commodity,
      currentAmount: value,
      accountName: account.name,
      balanceAmount: "",
      pad: undefined
    })));
    return sortBy(cloneDeep(items), item => item.accountName);
  }
)

export default function BatchBalance() {
  const dispatch = useAppDispatch();
  const accountStatus = useAppSelector((state) => state.accounts.status);
  const stateItems = useAppSelector(getFilteredItems);
  const accountSelectItems = [...useAppSelector(getAccountSelectItems())];
  const [accounts, acoountsHandler] = useListState<BalanceLineItem>(stateItems);

  const [maskCurrentAmount, setMaskCurrentAmount] = useLocalStorage({ key: 'tool/maskCurrentAmount', defaultValue: false });
  const [reflectOnUnbalancedAmount, setReflectOnUnbalancedAmount] = useLocalStorage({ key: 'tool/reflectOnUnbalancedAmount', defaultValue: true });
  

  useEffect(() => {
    if (accountStatus === LoadingState.NotReady) {
      dispatch(fetchAccounts());
    }
  }, [accountStatus, dispatch]);
  useEffect(() => {
    acoountsHandler.setState(stateItems);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stateItems])
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
          {accounts.map((account, idx) => <tr key={`${account.accountName}-${account.commodity}`}>
            <td>{account.accountName}</td>
            <td>{account.commodity}</td>
            <td><Amount mask={maskCurrentAmount} amount={account.currentAmount} currency={account.commodity} ></Amount></td>
            <td><Select
              searchable
              clearable
              placeholder="Pad to"
              data={accountSelectItems}
              value={account.pad}
              onChange={(e) => acoountsHandler.setItemProp(idx, 'pad', e ?? undefined)}
            /></td>
            <td>
              <TextInput
                value={account.balanceAmount}
                onChange={(e) => acoountsHandler.setItemProp(idx, 'balanceAmount', e.target.value)}
              ></TextInput>
            </td>
          </tr>)}
        </tbody>
      </Table>
      <Button>Submit</Button>
    </Container>

  );
}
