import BigNumber from 'bignumber.js';
import { useAtomValue } from 'jotai';
import { Search, X } from 'lucide-react';
import { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useLocation } from 'react-router';
import Amount from '@/components/Amount';
import { Skeleton } from '@/components/ui/skeleton';
import { cn } from '@/lib/utils';
import { accountAtom } from '@/states/account';
import { TreeTotal, treeTotals } from '@/utils/account-totals';

const GROUPS = [
  { type: 'Assets', label: 'SIDEBAR_ACCOUNTS_ASSETS' },
  { type: 'Liabilities', label: 'SIDEBAR_ACCOUNTS_LIABILITIES' },
] as const;

const ROW_CLASS = cn(
  'flex h-7 min-w-0 items-center justify-between gap-2.5 rounded-md px-2.5 text-[13px] text-foreground-2 outline-none',
  'hover:bg-accent focus-visible:ring-2 focus-visible:ring-sidebar-ring',
);
const ACTIVE_ROW_CLASS = 'bg-sidebar-accent text-sidebar-accent-foreground hover:bg-sidebar-accent';
const ICON_BUTTON_CLASS = cn(
  '-mr-1.5 flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none',
  'hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring',
);
const FILTER_INPUT_CLASS = cn(
  'h-7 w-full min-w-0 rounded-md border border-input bg-card px-2 text-[13px] outline-none placeholder:text-muted-foreground',
  'focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40',
);

/** The value of a group of accounts in the operating currency. */
function TotalAmount({ total }: { total: Pick<TreeTotal, 'number' | 'commodity'> | undefined }) {
  if (!total) return <span className="shrink-0 tabular-nums">—</span>;
  return (
    <span className="shrink-0">
      <Amount amount={total.number} currency={total.commodity} className={cn(total.number.isNegative() && 'text-negative')} />
    </span>
  );
}

/**
 * Desktop sidebar "Accounts" section (Actual Budget style): net total, then Assets / Liabilities with their subtotals and every
 * open account with its balance in the operating currency, with its sub-accounts as its page shows it. The totals are those of the
 * Accounts page: every account counts, a closed one that still holds money too. Rows link to the account page; the current one is
 * tinted. A search button toggles an inline filter. Expenses / Income are not listed.
 */
export function SidebarAccounts({ className }: { className?: string }) {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const accounts = useAtomValue(accountAtom);
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const activeAccount = pathname.startsWith('/accounts/') ? decodeURIComponent(pathname.slice('/accounts/'.length)) : undefined;
  const groups = useMemo(() => {
    const list = accounts.state === 'hasData' ? accounts.data : [];
    const needle = query.trim().toLowerCase();
    return GROUPS.map((group) => {
      const all = list.filter((account) => account.status === 'Open' && account.name.split(':')[0] === group.type).sort((a, b) => a.name.localeCompare(b.name));
      return { ...group, all, shown: needle ? all.filter((account) => account.name.toLowerCase().includes(needle)) : all };
    });
  }, [accounts, query]);
  const filtering = query.trim() !== '';
  // the values of the account tree, as the Accounts page shows them; a row shows the account with its sub-accounts
  const totals = useMemo(() => treeTotals(accounts.state === 'hasData' ? accounts.data : []), [accounts]);
  const net = useMemo(() => {
    const groupTotals = GROUPS.map((group) => totals.get(group.type)).filter((total) => total !== undefined);
    if (groupTotals.length === 0) return undefined;
    return { number: groupTotals.reduce((sum, total) => sum.plus(total.number), new BigNumber(0)), commodity: groupTotals[0].commodity };
  }, [totals]);

  const toggleSearch = () => {
    if (searching) {
      setSearching(false);
      setQuery('');
      return;
    }
    setSearching(true);
    requestAnimationFrame(() => inputRef.current?.focus());
  };

  return (
    <section aria-labelledby="sidebar-accounts-label" className={cn('flex min-h-0 flex-col', className)}>
      <div className="mx-2 mt-3 flex items-center justify-between border-t border-sidebar-border pt-2.5 pr-1 pb-1 pl-2.5">
        <h2 id="sidebar-accounts-label" className="text-xs font-normal text-muted-foreground">
          {t('SIDEBAR_ACCOUNTS')}
        </h2>
        <button
          type="button"
          onClick={toggleSearch}
          aria-pressed={searching}
          aria-label={t('SIDEBAR_ACCOUNTS_SEARCH')}
          title={t('SIDEBAR_ACCOUNTS_SEARCH')}
          className={ICON_BUTTON_CLASS}
        >
          {searching ? <X className="size-4" /> : <Search className="size-4" />}
        </button>
      </div>
      {searching && (
        <div className="px-2 pb-1.5">
          <input
            ref={inputRef}
            type="search"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === 'Escape') toggleSearch();
            }}
            placeholder={t('SIDEBAR_ACCOUNTS_FILTER')}
            aria-label={t('SIDEBAR_ACCOUNTS_FILTER')}
            className={FILTER_INPUT_CLASS}
          />
        </div>
      )}

      <nav aria-labelledby="sidebar-accounts-label" className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain px-2 pb-2">
        {accounts.state === 'loading' ? (
          <div className="flex flex-col gap-2 px-2.5 py-1">
            {[70, 55, 80, 60, 65].map((width) => (
              <Skeleton key={width} className="h-3.5" style={{ width: `${width}%` }} />
            ))}
          </div>
        ) : (
          <>
            {!filtering && (
              <Link to="/accounts" className={cn(ROW_CLASS, 'mb-1.5 font-semibold text-foreground')}>
                <span className="min-w-0 truncate">{t('SIDEBAR_ACCOUNTS_ALL')}</span>
                <TotalAmount total={net} />
              </Link>
            )}
            {groups.map((group) =>
              // a group without open accounts still shows its total, of closed accounts that hold money
              group.shown.length === 0 && (filtering || !totals.has(group.type)) ? null : (
                <div key={group.type} className="mt-1 flex flex-col">
                  <Link to="/accounts" className={cn(ROW_CLASS, 'font-semibold text-foreground')}>
                    <span className="min-w-0 truncate">{t(group.label)}</span>
                    <TotalAmount total={totals.get(group.type)} />
                  </Link>
                  {group.shown.map((account) => {
                    const short = account.name.split(':').slice(1).join(':') || account.name;
                    const balance = new BigNumber(account.amount_with_sub_accounts.calculated.number);
                    const active = account.name === activeAccount;
                    return (
                      <Link
                        key={account.name}
                        to={`/accounts/${account.name}`}
                        title={account.name}
                        aria-current={active ? 'page' : undefined}
                        className={cn(ROW_CLASS, active && ACTIVE_ROW_CLASS)}
                      >
                        <span className="min-w-0 truncate">{account.alias || short}</span>
                        <Amount
                          plain
                          amount={balance}
                          currency={account.amount_with_sub_accounts.calculated.commodity}
                          className={cn('shrink-0', balance.isNegative() && 'text-negative')}
                        />
                      </Link>
                    );
                  })}
                </div>
              ),
            )}
            {filtering && groups.every((group) => group.shown.length === 0) && (
              <p className="px-2.5 py-1 text-[13px] text-muted-foreground">{t('SIDEBAR_ACCOUNTS_NO_MATCH')}</p>
            )}
          </>
        )}
      </nav>
    </section>
  );
}
