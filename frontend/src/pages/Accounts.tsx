import { useAtomValue, useSetAtom } from 'jotai';
import { ChevronsDownUp, ChevronsUpDown, RefreshCw, Search, WalletMinimal, X } from 'lucide-react';
import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { EmptyState, LoadFailedState, PageHeader, PageShell } from '@/components/layout';
import { AccountListSkeleton } from '@/components/skeletons/accountListSkeleton';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { useInputState } from '@/hooks/use-input-state';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { ACCOUNTS_LINK } from '@/layout/nav-links';
import { setAccountsExpanded } from '@/components/layout/account-tree';
import AccountLine from '../components/AccountLine';
import Amount from '../components/Amount';
import { accountAtom, accountFetcher } from '../states/account';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { heldCommodities, treeTotals } from '../utils/account-totals';
import AccountTrie from '../utils/AccountTrie';

/** Conventional order of the five account types; anything else sorts after them alphabetically. */
const TYPE_ORDER = ['Assets', 'Liabilities', 'Equity', 'Income', 'Expenses'];

function collectGroupPaths(node: AccountTrie): string[] {
  return Object.values(node.children).flatMap((child) => (Object.keys(child.children).length > 0 ? [child.path, ...collectGroupPaths(child)] : []));
}

export default function Accounts() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('NAV_ACCOUNTS')} - ${ledgerTitle}`);

  const [filterKeyword, setFilterKeyword] = useInputState('');
  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });
  const accounts = useAtomValue(accountAtom);
  const refreshAccounts = useSetAtom(accountFetcher);
  const keyword = filterKeyword.trim().toLowerCase();

  useEffect(() => {
    setBreadcrumb([ACCOUNTS_LINK]);
  }, [setBreadcrumb]);

  const all = useMemo(() => (accounts.state === 'hasData' ? accounts.data : []), [accounts]);
  const closedCount = all.filter((it) => it.status !== 'Open').length;
  // the values of the whole tree: every account counts, closed ones included, whatever the page shows
  const totals = useMemo(() => treeTotals(all), [all]);

  const accountTrie = useMemo(() => {
    const trie = new AccountTrie();
    all
      .filter((it) => (hideClosedAccount ? it.status === 'Open' : true))
      .filter((it) => keyword === '' || it.name.toLowerCase().includes(keyword) || (it.alias?.toLowerCase() ?? '').includes(keyword))
      .forEach((it) => trie.insert(it));
    return trie;
  }, [all, hideClosedAccount, keyword]);

  const types = Object.keys(accountTrie.children).sort((a, b) => {
    const ia = TYPE_ORDER.indexOf(a);
    const ib = TYPE_ORDER.indexOf(b);
    return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib) || a.localeCompare(b);
  });
  const groupPaths = useMemo(() => collectGroupPaths(accountTrie), [accountTrie]);

  let content;
  if (accounts.state === 'loading') {
    content = <AccountListSkeleton />;
  } else if (accounts.state === 'hasError') {
    content = <LoadFailedState onRetry={refreshAccounts} />;
  } else if (types.length === 0) {
    content = (
      <EmptyState
        icon={WalletMinimal}
        title={keyword ? t('ledger.accounts.no_match_title', { keyword: filterKeyword.trim() }) : t('ledger.accounts.empty_title')}
        description={keyword ? t('ledger.accounts.no_match_description') : t('ledger.accounts.empty_description')}
        action={
          keyword && (
            <Button variant="outline" className="h-10 md:h-8" onClick={() => setFilterKeyword('')}>
              {t('ledger.common.clear_search')}
            </Button>
          )
        }
      />
    );
  } else {
    content = (
      <div className="flex flex-col gap-4">
        {types.map((type) => {
          const node = accountTrie.children[type];
          const total = totals.get(type);
          const multiple = heldCommodities(total).length > 1;
          return (
            <Card key={type} className="gap-0 py-0">
              <div className="flex min-h-12 items-center justify-between gap-3 px-4 py-2">
                <h2 className="flex items-baseline gap-2 text-sm font-medium">
                  {t(`ledger.account_type.${type}`, { defaultValue: type })}
                  <span className="text-xs font-normal text-muted-foreground tabular-nums">{countLeaves(node)}</span>
                </h2>
                <span className="flex items-baseline gap-1 text-sm font-semibold">
                  {multiple && <span aria-hidden>≈</span>}
                  {total && <Amount amount={total.number} currency={total.commodity} />}
                </span>
              </div>
              <div>
                {Object.keys(node.children)
                  .sort()
                  .map((child) => (
                    <AccountLine key={node.children[child].path} data={node.children[child]} spacing={0} forceExpand={keyword !== ''} totals={totals} />
                  ))}
              </div>
            </Card>
          );
        })}
      </div>
    );
  }

  return (
    <PageShell>
      <PageHeader
        title={t('NAV_ACCOUNTS')}
        description={
          accounts.state === 'hasData' ? t('ledger.accounts.description', { count: all.length, closed: closedCount }) : t('ledger.accounts.description_loading')
        }
      />

      <div className="flex flex-col gap-3 md:flex-row md:items-center">
        <div className="flex items-center gap-2 md:flex-1">
          <InputGroup className="h-10 flex-1 md:h-8 md:max-w-sm">
            <InputGroupAddon>
              <Search />
            </InputGroupAddon>
            <InputGroupInput
              type="text"
              enterKeyHint="search"
              placeholder={t('ledger.accounts.search_placeholder')}
              aria-label={t('ledger.accounts.search_placeholder')}
              value={filterKeyword}
              onChange={setFilterKeyword}
            />
            {filterKeyword && (
              <InputGroupAddon align="inline-end">
                <InputGroupButton size="icon-xs" aria-label={t('ledger.common.clear')} onClick={() => setFilterKeyword('')}>
                  <X />
                </InputGroupButton>
              </InputGroupAddon>
            )}
          </InputGroup>
          <Button variant="outline" className="h-10 md:hidden" aria-label={t('REFRESH')} onClick={() => refreshAccounts()}>
            <RefreshCw />
          </Button>
        </div>
        <div className="flex items-center justify-between gap-2 md:justify-end">
          <div className="flex min-h-10 items-center gap-2 md:min-h-8">
            <Switch id="hide-closed-accounts" checked={hideClosedAccount} onCheckedChange={setHideClosedAccount} />
            <Label htmlFor="hide-closed-accounts" className="font-normal text-muted-foreground">
              {t('accounts.hide_closed_accounts')}
            </Label>
          </div>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              className="size-10 px-0 md:h-8 md:w-auto md:px-2.5"
              aria-label={t('ledger.accounts.expand_all')}
              title={t('ledger.accounts.expand_all')}
              disabled={keyword !== '' || groupPaths.length === 0}
              onClick={() => setAccountsExpanded(groupPaths, true)}
            >
              <ChevronsUpDown />
              <span className="hidden md:inline">{t('ledger.accounts.expand_all')}</span>
            </Button>
            <Button
              variant="ghost"
              className="size-10 px-0 md:h-8 md:w-auto md:px-2.5"
              aria-label={t('ledger.accounts.collapse_all')}
              title={t('ledger.accounts.collapse_all')}
              disabled={keyword !== '' || groupPaths.length === 0}
              onClick={() => setAccountsExpanded(groupPaths, false)}
            >
              <ChevronsDownUp />
              <span className="hidden md:inline">{t('ledger.accounts.collapse_all')}</span>
            </Button>
            <Button variant="outline" className="hidden md:inline-flex" onClick={() => refreshAccounts()}>
              <RefreshCw data-icon="inline-start" />
              {t('REFRESH')}
            </Button>
          </div>
        </div>
      </div>

      {content}
    </PageShell>
  );
}

function countLeaves(node: AccountTrie): number {
  return Object.values(node.children).reduce((sum, child) => sum + (child.val ? 1 : 0) + countLeaves(child), 0);
}
