import { useListState, useLocalStorage } from '@mantine/hooks';
import { useEffect, useMemo } from 'react';
import Amount from '../../components/Amount';
import { loadable_unwrap } from '../../states';
import { accountAtom, accountFetcher, accountSelectItemsAtom } from '../../states/account';
import BigNumber from 'bignumber.js';
import { axiosInstance } from '../../global.ts';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { selectAtom } from 'jotai/utils';
import { toast } from 'sonner';
import { Switch } from '@/components/ui/switch.tsx';
import { Label } from '@/components/ui/label.tsx';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { Input } from '@/components/ui/input.tsx';
import { Combobox } from '@/components/ui/combobox.tsx';
import { Button } from '@/components/ui/button.tsx';
import { cn } from '@/lib/utils.ts';

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
          const isBalanced = new BigNumber(targetAccount.currentAmount).eq(new BigNumber(balanceAmount));
          accountsHandler.setItemProp(idx, 'error', !isBalanced);
          return;
        }
      }
    }
    accountsHandler.setItemProp(idx, 'error', false);
  };

  const onSave = async () => {
    const accountsToBalance = accounts
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
    toast.info(`Start balance ${accountsToBalance.length} Accounts`);

    try {
      await axiosInstance.post('/api/accounts/batch-balances', accountsToBalance);

      toast.success('Balance account successfully', {
        description: 'waiting page to refetch latest data',
      });
      refreshAccounts();
    } catch (e: any) {
      toast.error('Fail to Balance Account', {
        description: e?.response?.data ?? '',
      });
    }
  };
  return (
    <div>
      <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">Batch Balance</h1>
      <div className="flex items-center gap-2 mt-4">
        <Switch checked={maskCurrentAmount} onCheckedChange={setMaskCurrentAmount}></Switch>
        <Label>Mask Current Amount</Label>
        <Switch checked={reflectOnUnbalancedAmount} onCheckedChange={setReflectOnUnbalancedAmount}></Switch>
        <Label>Reflect on unbalanced amount</Label>
      </div>
      <div className="rounded-md border mt-4">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Account</TableHead>
              <TableHead>Commodity</TableHead>
              <TableHead className="text-right">Current Balance</TableHead>
              <TableHead>Pad Account</TableHead>
              <TableHead>Destination</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {accounts.map((account, idx) => (
              <TableRow key={`${account.accountName}-${account.commodity}`}>
                <TableCell>{account.accountName}</TableCell>
                <TableCell>{account.commodity}</TableCell>
                <TableCell className="text-right">
                  <Amount mask={maskCurrentAmount} amount={account.currentAmount} currency={account.commodity}></Amount>
                </TableCell>
                <TableCell>
                  <Combobox
                    placeholder="Pad to"
                    options={accountItems}
                    value={account.pad}
                    onChange={(e) => {
                      updateBalanceLineItem(idx, e ?? undefined, account.balanceAmount);
                    }}
                  />
                </TableCell>
                <TableCell>
                  <Input
                    type="number"
                    className={cn(account.error ? 'border-red-500' : '')}
                    value={account.balanceAmount}
                    onChange={(e) => {
                      updateBalanceLineItem(idx, account.pad ?? undefined, e.target.value);
                    }}
                  ></Input>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      <div className="flex justify-end mt-4">
        <Button disabled={accounts.filter((account) => account.balanceAmount.trim() !== '').length === 0} onClick={onSave}>
          Submit
        </Button>
      </div>
    </div>
  );
}
