import { useAtomValue, useSetAtom } from 'jotai';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { createAccountBalance } from '@/api/requests';
import { GroupCombobox } from '@/components/basic/GroupCombobox';
import { apiErrorMessage } from '@/lib/api-error';
import { accountFetcher, accountSelectItemsAtom } from '../states/account';
import Amount from './Amount';
import { Button } from './ui/button';
import { Field, FieldLabel } from './ui/field';
import { Input } from './ui/input';
import { Spinner } from './ui/spinner';

interface Props {
  /** The balance a `balance` on the account is checked against: with its sub-accounts. */
  currentAmount: string;
  /** The account has sub-accounts, which `currentAmount` includes. */
  includesSubAccounts?: boolean;
  commodity: string;
  accountName: string;
  /** Called after the balance directive was written. */
  onSaved?: () => void;
}

/**
 * Balance assertion / pad form for one commodity of an account. Stacked on mobile, one row on desktop.
 * With a pad account it writes a `pad` + `balance`, otherwise a plain `balance` check.
 */
export default function AccountBalanceCheckLine({ currentAmount, includesSubAccounts, commodity, accountName, onSaved }: Props) {
  const { t } = useTranslation();
  const id = useId();
  const [amount, setAmount] = useState('');
  const [padAccount, setPadAccount] = useState<string>('');
  const [saving, setSaving] = useState(false);
  const refreshAccounts = useSetAtom(accountFetcher);
  const accountItems = useAtomValue(accountSelectItemsAtom);

  const onSave = async () => {
    setSaving(true);
    try {
      await createAccountBalance({
        account_name: accountName,
        amount: { number: amount, commodity: commodity },
        pad: padAccount,
        type: padAccount ? 'Pad' : 'Check',
      });
      toast.success(t('ledger.balance.saved'));
      setAmount('');
      setPadAccount('');
      refreshAccounts();
      onSaved?.();
    } catch (e: unknown) {
      toast.error(t('ledger.balance.failed'), { description: await apiErrorMessage(e), duration: 10000 });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="grid gap-3 rounded-lg border p-3 md:grid-cols-[10rem_minmax(0,1fr)_minmax(0,12rem)_auto] md:items-end">
      <div className="flex items-baseline justify-between gap-2 md:flex-col md:items-start md:gap-0.5">
        <span className="text-sm font-medium">{commodity}</span>
        <span className="text-xs text-muted-foreground">
          {includesSubAccounts ? t('ledger.balance.current_with_sub_accounts') : t('ledger.balance.current')}{' '}
          <Amount className="text-foreground" amount={currentAmount} currency={commodity} />
        </span>
      </div>
      <Field className="gap-1.5">
        <FieldLabel htmlFor={`${id}-pad`} className="text-xs text-muted-foreground">
          {t('ledger.balance.pad_from')}
        </FieldLabel>
        <GroupCombobox
          id={`${id}-pad`}
          className="h-10 md:h-8"
          placeholder={t('ledger.balance.pad_placeholder')}
          options={accountItems}
          value={padAccount}
          onChange={(value) => setPadAccount(value ?? '')}
        />
      </Field>
      <Field className="gap-1.5">
        <FieldLabel htmlFor={`${id}-amount`} className="text-xs text-muted-foreground">
          {t('ledger.balance.amount', { commodity })}
        </FieldLabel>
        <Input
          id={`${id}-amount`}
          className="h-10 tabular-nums md:h-8"
          type="number"
          step="any"
          placeholder="0.00"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
        />
      </Field>
      <Button className="h-10 md:h-8" onClick={onSave} disabled={amount.length === 0 || saving}>
        {saving && <Spinner data-icon="inline-start" />}
        {padAccount ? t('ledger.balance.pad') : t('ledger.balance.check')}
      </Button>
    </div>
  );
}
