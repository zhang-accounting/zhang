import BigNumber from 'bignumber.js';
import { useAtomValue, useSetAtom } from 'jotai';
import { selectAtom } from 'jotai/utils';
import { RotateCcw, Search, SquareStack } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { createBatchBalance } from '@/api/requests';
import Amount from '@/components/Amount';
import { GroupCombobox } from '@/components/basic/GroupCombobox';
import { EmptyState, PageHeader, PageShell, useIsMobile } from '@/components/layout';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group';
import { Skeleton } from '@/components/ui/skeleton';
import { Spinner } from '@/components/ui/spinner';
import { Switch } from '@/components/ui/switch';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { batchBalanceRows, replacedBalancesText, subAccountsFirst } from '@/utils/balance-check';
import { useListState } from '@/hooks/use-list-state';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { apiErrorMessage } from '@/lib/api-error';
import { TOOLS_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { loadable_unwrap } from '@/states';
import { accountAtom, accountFetcher, accountSelectItemsAtom } from '@/states/account';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

interface BalanceLineItem {
  commodity: string;
  /** The balance a `balance` on the account is checked against: with its sub-accounts. */
  currentAmount: string;
  accountName: string;
  /** The account has sub-accounts, which `currentAmount` includes. */
  hasSubAccounts: boolean;

  balanceAmount: string;
  pad?: string;
}

/** A plain check (no pad) whose typed amount differs from the current balance. */
function isMismatch(item: BalanceLineItem) {
  if (item.pad || item.balanceAmount.trim() === '') return false;
  const typed = new BigNumber(item.balanceAmount);
  return !typed.isNaN() && !new BigNumber(item.currentAmount).eq(typed);
}

export default function BatchBalance() {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('tools.batch_balance_title')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([TOOLS_LINK, { label: 'tools.batch_balance_title', uri: '/tools/batch-balance' }]);
  }, [setBreadcrumb]);

  const stateItems = useAtomValue(
    useMemo(
      () =>
        selectAtom(accountAtom, (val) =>
          loadable_unwrap(val, [], (data) => {
            // stable order: the API returns accounts in arbitrary order, rows must not jump on reload
            return batchBalanceRows(data).map((row) => ({
              commodity: row.commodity,
              currentAmount: row.currentAmount,
              accountName: row.accountName,
              hasSubAccounts: row.includesSubAccounts,
              balanceAmount: '',
              pad: undefined,
            }));
          }),
        ),
      [],
    ),
  );
  const accountsLoading = useAtomValue(accountAtom).state === 'loading';

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
  const [keyword, setKeyword] = useState('');
  const [submitting, setSubmitting] = useState(false);

  // Background refreshes (SSE ledger reloads) keep what the user has typed; only a successful submit clears the form.
  // `accountAtom` is a loadable, so every refresh passes through `loading` (empty items) first: ignore that phase.
  const resetOnNextRefresh = useRef(false);
  useEffect(() => {
    if (accountsLoading) return;
    if (resetOnNextRefresh.current) {
      resetOnNextRefresh.current = false;
      accountsHandler.setState(stateItems);
      return;
    }
    accountsHandler.setState((current) => {
      const key = (it: BalanceLineItem) => `${it.accountName}\u0000${it.commodity}`;
      const typed = new Map(current.filter((it) => it.balanceAmount !== '' || it.pad).map((it) => [key(it), it]));
      return stateItems.map((item) => {
        const previous = typed.get(key(item));
        if (!previous) return item;
        return { ...item, balanceAmount: previous.balanceAmount, pad: previous.pad };
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stateItems, accountsLoading]);

  const updateBalanceLineItem = (idx: number, padAccount: string | undefined, balanceAmount: string) => {
    accountsHandler.setItemProp(idx, 'pad', padAccount);
    accountsHandler.setItemProp(idx, 'balanceAmount', balanceAmount);
  };

  // Derived (not stored) so that toggling "Reflect" off clears every flag and the count at once, and on brings them back.
  const hasMismatch = (account: BalanceLineItem) => reflectOnUnbalancedAmount && isMismatch(account);
  const filledCount = accounts.filter((account) => account.balanceAmount.trim() !== '').length;
  const mismatchCount = accounts.filter(hasMismatch).length;

  const onSave = async () => {
    // sub-accounts first: a parent's `balance` covers them, so it must come after their pads
    const accountsToBalance = subAccountsFirst(
      accounts
        .filter((account) => account.balanceAmount.trim() !== '')
        .map((account) => ({
          type: account.pad ? ('Pad' as const) : ('Check' as const),
          account_name: account.accountName,
          amount: {
            number: account.balanceAmount,
            commodity: account.commodity,
          },
          pad: account.pad ?? '',
        })),
    );
    toast.info(t('batch_balance.start_toast', { count: accountsToBalance.length }));
    setSubmitting(true);
    try {
      const res = await createBatchBalance(accountsToBalance);

      // a check of a beancount ledger replaces the balance it wrote earlier today
      const replaced = replacedBalancesText(res.data.data.replaced, (it) => t('ledger.balance.replaced', it));
      toast.success(t('batch_balance.success_toast'), {
        description: replaced ? `${replaced}\n${t('batch_balance.success_toast_description')}` : t('batch_balance.success_toast_description'),
        duration: replaced ? 10000 : undefined,
      });
      resetOnNextRefresh.current = true;
      accountsHandler.setState(stateItems);
      refreshAccounts();
    } catch (e) {
      toast.error(t('batch_balance.failed_toast'), { description: await apiErrorMessage(e) });
    } finally {
      setSubmitting(false);
    }
  };

  const visibleRows = useMemo(() => {
    const needle = keyword.trim().toLowerCase();
    return accounts
      .map((account, idx) => ({ account, idx }))
      .filter(({ account }) => needle === '' || account.accountName.toLowerCase().includes(needle) || account.commodity.toLowerCase().includes(needle));
  }, [accounts, keyword]);

  const amountInput = (account: BalanceLineItem, idx: number, className?: string) => (
    <Input
      id={`batch-balance-${idx}`}
      type="number"
      step="any"
      placeholder={isMobile ? t('batch_balance.actual') : undefined}
      aria-label={`${t('batch_balance.actual')} · ${account.accountName} ${account.commodity}`}
      aria-invalid={hasMismatch(account) || undefined}
      className={cn('text-right tabular-nums', className)}
      value={account.balanceAmount}
      onChange={(e) => {
        updateBalanceLineItem(idx, account.pad ?? undefined, e.target.value);
      }}
    />
  );
  const padSelect = (account: BalanceLineItem, idx: number, className?: string) => (
    <GroupCombobox
      placeholder={isMobile ? t('batch_balance.pad_from') : t('batch_balance.pad_placeholder')}
      options={accountItems}
      value={account.pad}
      className={className}
      onChange={(e) => {
        updateBalanceLineItem(idx, e ?? undefined, account.balanceAmount);
      }}
    />
  );

  return (
    <PageShell>
      <PageHeader title={t('tools.batch_balance_title')} description={t('batch_balance.description')} />

      <Card size="sm">
        <CardContent>
          <FieldGroup className="gap-4 md:flex-row md:items-center md:gap-6">
            <Field orientation="horizontal" className="md:w-auto">
              <Switch id="batch-balance-mask" checked={maskCurrentAmount} onCheckedChange={setMaskCurrentAmount} />
              <FieldContent>
                <FieldLabel htmlFor="batch-balance-mask">{t('batch_balance.mask')}</FieldLabel>
                <FieldDescription className="text-xs">{t('batch_balance.mask_description')}</FieldDescription>
              </FieldContent>
            </Field>
            <Field orientation="horizontal" className="md:w-auto">
              <Switch id="batch-balance-reflect" checked={reflectOnUnbalancedAmount} onCheckedChange={setReflectOnUnbalancedAmount} />
              <FieldContent>
                <FieldLabel htmlFor="batch-balance-reflect">{t('batch_balance.reflect')}</FieldLabel>
                <FieldDescription className="text-xs">{t('batch_balance.reflect_description')}</FieldDescription>
              </FieldContent>
            </Field>
            <InputGroup className="h-10 md:ml-auto md:h-8 md:w-64">
              <InputGroupAddon>
                <Search />
              </InputGroupAddon>
              <InputGroupInput
                value={keyword}
                onChange={(event) => setKeyword(event.target.value)}
                placeholder={t('batch_balance.filter_placeholder')}
                aria-label={t('batch_balance.filter_placeholder')}
              />
            </InputGroup>
          </FieldGroup>
        </CardContent>
      </Card>

      {accountsLoading && accounts.length === 0 ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 5 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : visibleRows.length === 0 ? (
        <EmptyState
          icon={SquareStack}
          title={accounts.length === 0 ? t('batch_balance.empty_title') : t('batch_balance.no_match_title')}
          description={accounts.length === 0 ? t('batch_balance.empty_description') : undefined}
        />
      ) : isMobile ? (
        <ul className="flex flex-col divide-y overflow-hidden rounded-xl border bg-card">
          {visibleRows.map(({ account, idx }) => (
            <li
              key={`${account.accountName}-${account.commodity}`}
              className={cn('flex flex-col gap-2.5 p-3', account.balanceAmount.trim() !== '' && 'bg-primary/3')}
            >
              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-sm font-medium">{account.accountName}</span>
                  <Badge variant="secondary" className="shrink-0">
                    {account.commodity}
                  </Badge>
                </div>
                <span className="flex shrink-0 flex-col items-end text-sm text-muted-foreground tabular-nums">
                  <Amount mask={maskCurrentAmount} amount={account.currentAmount} currency={account.commodity} />
                  {account.hasSubAccounts && <span className="text-xs">{t('batch_balance.with_sub_accounts')}</span>}
                </span>
              </div>
              <div className="grid grid-cols-2 gap-2">
                {amountInput(account, idx, 'h-10')}
                {padSelect(account, idx, 'h-10')}
              </div>
              {hasMismatch(account) && <FieldError className="text-xs">{t('batch_balance.mismatch')}</FieldError>}
            </li>
          ))}
        </ul>
      ) : (
        <div className="overflow-hidden rounded-xl border bg-card">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead>{t('batch_balance.account')}</TableHead>
                <TableHead className="w-24">{t('batch_balance.commodity')}</TableHead>
                <TableHead className="w-44 text-right">{t('batch_balance.current')}</TableHead>
                <TableHead className="w-64">{t('batch_balance.pad_from')}</TableHead>
                <TableHead className="w-48 text-right">{t('batch_balance.actual')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visibleRows.map(({ account, idx }) => (
                <TableRow key={`${account.accountName}-${account.commodity}`} className={cn(account.balanceAmount.trim() !== '' && 'bg-primary/3')}>
                  <TableCell className="max-w-0 truncate font-medium" title={account.accountName}>
                    {account.accountName}
                  </TableCell>
                  <TableCell>
                    <Badge variant="secondary">{account.commodity}</Badge>
                  </TableCell>
                  <TableCell className="text-right tabular-nums">
                    <Amount mask={maskCurrentAmount} amount={account.currentAmount} currency={account.commodity} />
                    {account.hasSubAccounts && <div className="text-xs text-muted-foreground">{t('batch_balance.with_sub_accounts')}</div>}
                  </TableCell>
                  <TableCell>{padSelect(account, idx)}</TableCell>
                  <TableCell>
                    {amountInput(account, idx)}
                    {hasMismatch(account) && <div className="mt-1 text-right text-xs text-destructive">{t('batch_balance.mismatch')}</div>}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      <div
        className={cn(
          'sticky bottom-[calc(4.5rem+env(safe-area-inset-bottom))] z-20 flex items-center gap-2 rounded-xl border p-2 pl-4 shadow-sm md:bottom-4',
          'bg-background/95 backdrop-blur supports-backdrop-filter:bg-background/80',
        )}
      >
        <div className="flex min-w-0 flex-1 flex-col text-sm sm:flex-row sm:items-center sm:gap-3">
          <span className="truncate font-medium tabular-nums">{t('batch_balance.filled', { count: filledCount, total: accounts.length })}</span>
          {mismatchCount > 0 && (
            <span className="truncate text-xs text-destructive tabular-nums">{t('batch_balance.mismatch_count', { count: mismatchCount })}</span>
          )}
        </div>
        <Button
          variant="ghost"
          className="h-10 md:h-8"
          disabled={filledCount === 0 || submitting}
          onClick={() => accountsHandler.setState(stateItems)}
          aria-label={t('batch_balance.reset')}
        >
          <RotateCcw />
          <span className="hidden sm:inline">{t('batch_balance.reset')}</span>
        </Button>
        <Button className="h-10 md:h-8" disabled={filledCount === 0 || submitting} onClick={onSave}>
          {submitting && <Spinner />}
          {t('batch_balance.submit')}
        </Button>
      </div>
    </PageShell>
  );
}
