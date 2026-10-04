import { useAtomValue, useSetAtom } from 'jotai';
import { ChartLine, CircleAlert, Cog, FileStack, NotebookText, WalletMinimal } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useParams, useSearchParams } from 'react-router-dom';
import { useAsync } from 'react-use';
import { retrieveAccountBalance, retrieveAccountDocuments, retrieveAccountInfo, retrieveAccountJournals } from '@/api/requests';
import { EmptyState, PageHeader, PageShell, ResponsiveList } from '@/components/layout';
import { PagePagination } from '@/components/layout/PagePagination';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { useDateFormat } from '@/components/layout/use-date-format';
import { Badge } from '@/components/ui/badge';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { Skeleton } from '@/components/ui/skeleton';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { ACCOUNTS_LINK } from '@/layout/nav-links';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import { balanceCheckRows } from '../utils/balance-check';
import { AccountBalanceHistoryGraph } from '../components/AccountBalanceHistoryGraph';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import PayeeNarration from '../components/basic/PayeeNarration';
import { ImageLightBox } from '../components/ImageLightBox';
import DocumentPreview from '../components/journalPreview/DocumentPreview';
import Section from '../components/Section';
import { breadcrumbAtom, titleAtom } from '../states/basic';

const TABS = ['journals', 'documents', 'history', 'settings'] as const;
type TabKey = (typeof TABS)[number];

function SingleAccount() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { accountName } = useParams();
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedTab = searchParams.get('tab') as TabKey | null;
  const tab: TabKey = requestedTab && TABS.includes(requestedTab) ? requestedTab : 'journals';
  const [reloadKey, setReloadKey] = useState(0);
  const tabsScroller = useRef<HTMLDivElement>(null);
  const reload = () => setReloadKey((key) => key + 1);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${accountName} | ${t('NAV_ACCOUNTS')} - ${ledgerTitle}`);

  useEffect(() => {
    setBreadcrumb([ACCOUNTS_LINK, { label: accountName ?? '', uri: `/accounts/${accountName}`, noTranslate: true }]);
  }, [accountName, setBreadcrumb]);

  const info = useAsync(async () => {
    if (!accountName) return undefined;
    return (await retrieveAccountInfo({ account_name: accountName })).data.data;
  }, [accountName, reloadKey]);
  const account = info.value;

  // Keep the active tab visible in the horizontally scrolling tab strip on mobile.
  useEffect(() => {
    tabsScroller.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [tab]);

  const onTabChange = (value: unknown) => {
    const next = new URLSearchParams(searchParams);
    if (value === 'journals') next.delete('tab');
    else next.set('tab', String(value));
    setSearchParams(next, { replace: true });
  };

  if (info.error) {
    return (
      <PageShell>
        <EmptyState
          icon={CircleAlert}
          title={t('ledger.account.not_found', { name: accountName })}
          description={String(info.error)}
          action={
            <Link to="/accounts" className={cn(buttonVariants({ variant: 'outline' }), 'h-10 md:h-8')}>
              {t('ledger.account.back_to_accounts')}
            </Link>
          }
        />
      </PageShell>
    );
  }

  // the page shows the account with its sub-accounts, as the account tree does
  const subAccounts = account?.has_sub_accounts ?? false;
  const total = subAccounts ? account?.amount_with_sub_accounts : account?.amount;
  const details = Object.entries(total?.detail ?? {});
  const multiple = details.length > 1;
  // what a `balance` on the account is checked against: with its sub-accounts
  const checkRows = account ? balanceCheckRows(account) : [];

  return (
    <PageShell>
      <PageHeader
        title={account ? (account.alias ?? account.name) : (accountName ?? '')}
        description={
          <span className="flex flex-wrap items-center gap-2">
            {account?.alias && <span className="break-all">{account.name}</span>}
            {account ? (
              <Badge variant={account.status === 'Open' ? 'secondary' : 'outline'} className="font-normal">
                {account.status === 'Open' ? t('ledger.accounts.open') : t('ledger.accounts.closed')}
              </Badge>
            ) : (
              <span className="inline-block h-5 w-14 animate-pulse rounded-full bg-muted" />
            )}
            {account && <span>{t(`ledger.account_type.${account.name.split(':')[0]}`, { defaultValue: account.name.split(':')[0] })}</span>}
          </span>
        }
        actions={
          <div className="flex flex-col gap-0.5 sm:items-end">
            <span className="text-xs font-medium text-muted-foreground">
              {subAccounts ? t('ledger.account.balance_with_sub_accounts') : t('ledger.account.balance')}
            </span>
            {account && total ? (
              <span className="flex items-baseline gap-1 text-2xl font-semibold tracking-tight">
                {multiple && <span className="text-muted-foreground">≈</span>}
                <Amount amount={total.calculated.number} currency={total.calculated.commodity} />
              </span>
            ) : (
              <Skeleton className="h-8 w-40" />
            )}
            {multiple && (
              <span className="flex flex-wrap gap-x-3 gap-y-0.5 text-sm text-muted-foreground sm:justify-end">
                {details.map(([commodity, amount]) => (
                  <Amount key={commodity} amount={amount} currency={commodity} />
                ))}
              </span>
            )}
            {account && subAccounts && (
              <span className="text-xs text-muted-foreground">
                {t('ledger.account.own_balance')} <Amount amount={account.amount.calculated.number} currency={account.amount.calculated.commodity} />
              </span>
            )}
          </div>
        }
      />

      <Tabs value={tab} onValueChange={onTabChange} className="gap-4">
        <div ref={tabsScroller} className="-mx-4 overflow-x-auto px-4 md:mx-0 md:px-0 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          <TabsList variant="line" className="h-10 w-max justify-start gap-1 border-b md:h-9 md:w-full">
            <TabsTrigger value="journals" className="flex-none px-3">
              <NotebookText /> {t('ledger.account.tab_journals')}
            </TabsTrigger>
            <TabsTrigger value="documents" className="flex-none px-3">
              <FileStack /> {t('ledger.account.tab_documents')}
            </TabsTrigger>
            <TabsTrigger value="history" className="flex-none px-3">
              <ChartLine /> {t('ledger.account.tab_history')}
            </TabsTrigger>
            <TabsTrigger value="settings" className="flex-none px-3">
              <Cog /> {t('ledger.account.tab_settings')}
            </TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="journals">
          <AccountJournals accountName={accountName ?? ''} reloadKey={reloadKey} />
        </TabsContent>
        <TabsContent value="documents">
          <AccountDocuments accountName={accountName ?? ''} subAccounts={subAccounts} />
        </TabsContent>
        <TabsContent value="history">
          <AccountHistory accountName={accountName ?? ''} subAccounts={subAccounts} reloadKey={reloadKey} />
        </TabsContent>
        <TabsContent value="settings">
          <Section title={t('ledger.balance.title')} description={t('ledger.balance.description')}>
            {account ? (
              checkRows.length === 0 ? (
                <EmptyState icon={WalletMinimal} title={t('ledger.balance.no_commodities')} />
              ) : (
                <div className="flex flex-col gap-3">
                  {checkRows.map((row) => (
                    <AccountBalanceCheckLine
                      key={row.commodity}
                      currentAmount={row.currentAmount}
                      includesSubAccounts={row.includesSubAccounts}
                      commodity={row.commodity}
                      accountName={account.name}
                      onSaved={reload}
                    />
                  ))}
                </div>
              )
            ) : (
              <Skeleton className="h-16 w-full" />
            )}
          </Section>
        </TabsContent>
      </Tabs>
    </PageShell>
  );
}

export default SingleAccount;

/** Rows of a page of the journal, as on the Journals page. */
const JOURNAL_PAGE_SIZE = 100;

function AccountJournals({ accountName, reloadKey }: { accountName: string; reloadKey: number }) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const [page, setPage] = useState(1);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setPage(1), [accountName]);
  const journals = useAsync(async () => {
    const response = await retrieveAccountJournals({ account_name: accountName, page, size: JOURNAL_PAGE_SIZE });
    // the number of rows of all the pages
    const total = Number(response.headers.get('X-Total-Count') ?? response.data.data.length);
    return { rows: response.data.data, totalPages: Math.ceil(total / JOURNAL_PAGE_SIZE) };
  }, [accountName, reloadKey, page]);
  type Row = NonNullable<typeof journals.value>['rows'][number];
  const onPage = (next: number) => {
    setPage(next);
    list.current?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  };

  // The journal is that of the account and its sub-accounts: the balance column is their running balance, and the rows
  // of a sub-account name it. A balance assertion adds nothing: its row shows the amount it asserted instead, red when
  // it failed, and the balance it was checked against in the balance column.
  const change = (item: Row, className?: string) =>
    item.asserted ? (
      <span title={t('ledger.preview.balance_amount')} className={cn('text-muted-foreground', !item.passed && 'text-destructive', className)}>
        = <Amount amount={item.asserted.number} currency={item.asserted.commodity} />
      </span>
    ) : (
      <Amount className={className} tone signed amount={item.inferred_unit.number} currency={item.inferred_unit.commodity} />
    );
  const failed = (item: Row) =>
    item.passed === false && (
      <Badge variant="destructive" className="shrink-0">
        {t('ledger.journal.check_failed')}
      </Badge>
    );
  // the posting's sub-account, relative to the page's account; nothing for the account itself
  const prefix = `${accountName}:`;
  const subAccount = (item: Row) =>
    item.account.startsWith(prefix) && (
      <span title={item.account} className="min-w-0 truncate rounded-md bg-muted px-1.5 py-0.5 text-xs text-foreground-2">
        {item.account.slice(prefix.length)}
      </span>
    );
  const hasSubAccountRows = (journals.value?.rows ?? []).some((item) => item.account.startsWith(prefix));

  if (journals.error) return <EmptyState icon={CircleAlert} title={t('ledger.common.load_failed')} description={String(journals.error)} />;

  return (
    <div ref={list} className="flex scroll-mt-4 flex-col gap-2">
      <div className="flex justify-end">
        <OpenInExplore name="accounts.journal" params={{ account: accountName }} />
      </div>
      <ResponsiveList<Row>
        items={journals.value?.rows ?? []}
        loading={journals.loading}
        getKey={(item, index) => `${item.trx_id}-${index}`}
        empty={<EmptyState icon={NotebookText} title={t('ledger.account.no_journals')} description={t('ledger.account.no_journals_description')} />}
        columns={[
          {
            key: 'date',
            header: t('ledger.account.col_date'),
            className: 'w-40 pl-4 text-muted-foreground tabular-nums',
            cell: (item) => fmt.dateTime(new Date(item.datetime)),
          },
          {
            key: 'payee',
            header: t('ledger.journals.col_description'),
            className: 'w-full max-w-0',
            cell: (item) => (
              <span className="flex min-w-0 items-center gap-2">
                <PayeeNarration payee={item.payee} narration={item.narration} />
                {failed(item)}
              </span>
            ),
          },
          ...(hasSubAccountRows
            ? [
                {
                  key: 'account',
                  header: t('ledger.account.col_sub_account'),
                  className: 'max-w-48',
                  cell: (item: Row) => <span className="flex min-w-0">{subAccount(item)}</span>,
                },
              ]
            : []),
          {
            key: 'change',
            header: t('ledger.account.col_change'),
            className: 'text-right',
            cell: (item) => change(item),
          },
          {
            key: 'after',
            header: t('ledger.account.col_balance'),
            className: 'pr-4 text-right text-muted-foreground',
            cell: (item) => <Amount amount={item.account_after.number} currency={item.account_after.commodity} />,
          },
        ]}
        renderCard={(item) => (
          <div className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 flex-col gap-0.5">
              <span className="flex min-w-0 items-center gap-2">
                <span className="truncate text-sm font-medium">{item.narration || item.payee || '—'}</span>
                {failed(item)}
              </span>
              {subAccount(item) && <span className="flex min-w-0">{subAccount(item)}</span>}
              <span className="truncate text-xs text-muted-foreground">
                {[item.narration ? item.payee : null, fmt.dateTime(new Date(item.datetime))].filter(Boolean).join(' · ')}
              </span>
            </div>
            <div className="flex shrink-0 flex-col items-end gap-0.5 text-sm">
              {change(item, 'font-semibold')}
              <Amount className="text-xs text-muted-foreground" amount={item.account_after.number} currency={item.account_after.commodity} />
            </div>
          </div>
        )}
      />
      {journals.value && <PagePagination page={page} totalPages={journals.value.totalPages} onPageChange={onPage} />}
    </div>
  );
}

function AccountDocuments({ accountName, subAccounts }: { accountName: string; subAccounts: boolean }) {
  const { t } = useTranslation();
  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);
  const [reloadKey, setReloadKey] = useState(0);
  const documents = useAsync(async () => (await retrieveAccountDocuments({ account_name: accountName })).data.data, [accountName, reloadKey]);

  if (documents.error) return <EmptyState icon={CircleAlert} title={t('ledger.common.load_failed')} description={String(documents.error)} />;

  return (
    <Section
      title={t('ledger.account.documents_title', { count: documents.value?.length ?? 0 })}
      description={subAccounts ? t('ledger.account.documents_description_with_sub_accounts') : t('ledger.account.documents_description')}
      rightSection={<OpenInExplore name="accounts.documents" params={{ account: accountName }} iconOnly />}
    >
      <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 md:gap-3 lg:grid-cols-5">
        <AccountDocumentUpload id={accountName} type="account" onUploaded={() => setReloadKey((key) => key + 1)} />
        {documents.loading && !documents.value
          ? Array.from({ length: 3 }, (_, index) => <Skeleton key={index} className="aspect-square rounded-lg" />)
          : (documents.value ?? []).map((document, idx) => <DocumentPreview onClick={(path) => setLightboxSrc(path)} key={idx} filename={document.path} />)}
      </div>
    </Section>
  );
}

function AccountHistory({ accountName, subAccounts, reloadKey }: { accountName: string; subAccounts: boolean; reloadKey: number }) {
  const { t } = useTranslation();
  const history = useAsync(async () => (await retrieveAccountBalance({ account_name: accountName })).data.data, [accountName, reloadKey]);
  return (
    <Section
      title={t('ledger.account.history_title')}
      description={subAccounts ? t('ledger.account.history_description_with_sub_accounts') : t('ledger.account.history_description')}
      rightSection={<OpenInExplore name="accounts.balance_history" params={{ account: accountName }} iconOnly />}
    >
      {history.error ? (
        <EmptyState icon={CircleAlert} title={t('ledger.common.load_failed')} description={String(history.error)} />
      ) : history.value ? (
        <AccountBalanceHistoryGraph data={history.value.balance} />
      ) : (
        <Skeleton className="h-56 w-full md:h-80" />
      )}
    </Section>
  );
}
