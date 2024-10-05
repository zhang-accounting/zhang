import { useState } from 'react';
import { showNotification } from '@mantine/notifications';
import { accountFetcher, accountSelectItemsAtom } from '../states/account';
import Amount from './Amount';
import { useAtomValue } from 'jotai';
import { useSetAtom } from 'jotai/index';
import { axiosInstance } from '../global.ts';
import { TableRow, TableCell } from './ui/table.tsx';
import { Combobox } from './ui/combobox.tsx';
import { Input } from './ui/input.tsx';
import { Group } from '@mantine/core';
import { Button } from './ui/button.tsx';

interface Props {
  currentAmount: string;
  commodity: string;
  accountName: string;
}

export default function AccountBalanceCheckLine({ currentAmount, commodity, accountName }: Props) {
  const [amount, setAmount] = useState('');
  const [padAccount, setPadAccount] = useState<string>('');
  const refreshAccounts = useSetAtom(accountFetcher);
  const accountItems = useAtomValue(accountSelectItemsAtom);

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

  const submitCheck = () => {
    onSave();
    setAmount('');
  };
  return (
    <>
      <TableRow>
        <TableCell>{commodity}</TableCell>
        <TableCell>
          <Amount amount={currentAmount} currency={commodity} />
        </TableCell>
        <TableCell>{}</TableCell>
        <TableCell>
          <Combobox placeholder="Pad to" options={accountItems} value={padAccount} onChange={(value) => setPadAccount(value ?? '')} />
        </TableCell>
        <TableCell>
          <div className="flex items-center gap-2">
            <Input type="number" placeholder={`Balanced ${commodity} Amount`} value={amount}
                       onChange={(e) => setAmount(e.target.value)}></Input>
            <Button size="sm" onClick={submitCheck} disabled={amount.length === 0}>
              {padAccount ? 'Pad' : 'Balance'}
            </Button>
          </div>
        </TableCell>
      </TableRow>
    </>
  );
}
