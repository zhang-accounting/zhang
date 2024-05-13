import { Autocomplete, Button, Group, Table, TextInput } from '@mantine/core';
import { useState } from 'react';
import { axiosInstance } from '../index';
import { showNotification } from '@mantine/notifications';
import { useAppDispatch, useAppSelector } from '../states';
import { accountsSlice, getAccountSelectItems } from '../states/account';
import Amount from './Amount';

interface Props {
  currentAmount: string;
  commodity: string;
  accountName: string;
}

export default function AccountBalanceCheckLine({ currentAmount, commodity, accountName }: Props) {
  const [amount, setAmount] = useState('');
  const [padAccount, setPadAccount] = useState<string>('');
  const dispatch = useAppDispatch();

  const accountItems = [...useAppSelector(getAccountSelectItems())];
  console.log('accountItems', accountItems);

  const onSave = async () => {
    try {
      await axiosInstance.post(`/api/accounts/${accountName}/balances`, {
        type: padAccount ? 'Pad' : 'Check',
        account_name: accountName,
        amount: {
          number: amount,
          commodity: commodity,
        },
        pad: padAccount,
      });
      showNotification({
        title: 'Balance account successfully',
        message: '',
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

  const submitCheck = () => {
    onSave();
    setAmount('');
  };
  return (
    <>
      <Table.Tr>
        <Table.Td>{commodity}</Table.Td>
        <Table.Td>
          <Amount amount={currentAmount} currency={commodity} />
        </Table.Td>
        <Table.Td>{}</Table.Td>
        <Table.Td>
          <Autocomplete placeholder="Pad to" data={accountItems} value={padAccount}
                        onChange={setPadAccount} />
        </Table.Td>
        <Table.Td>
          <Group gap={'xs'}>
            <TextInput placeholder={`Balanced ${commodity} Amount`} value={amount}
                       onChange={(e) => setAmount(e.target.value)}></TextInput>
            <Button size="sm" onClick={submitCheck} disabled={amount.length === 0}>
              {padAccount ? 'Pad' : 'Balance'}
            </Button>
          </Group>
        </Table.Td>
      </Table.Tr>
    </>
  );
}
