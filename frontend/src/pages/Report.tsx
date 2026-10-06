import BigNumber from 'bignumber.js';
import { ArrowDownLeft, ArrowUpRight, CircleAlert, Hash, Landmark, ReceiptText } from 'lucide-react';
import { OpReturnType } from 'openapi-typescript-fetch';
import { type ReactNode, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { retrieveStatisticByAccountType, retrieveStatisticGraph, retrieveStatisticSummary } from '@/api/requests';
import { operations } from '@/api/schemas';
import { EmptyState, PageHeader, PageShell, RefreshingLabel, ResponsiveList } from '@/components/layout';
import { DateRangePicker, DateRangePreset, DateRangeValue } from '@/components/layout/DateRangePicker';
import { useDateFormat } from '@/components/layout/use-date-format';
import { activityAnchor, monthOf, useRecentJournals } from '@/components/layout/use-ledger-activity';
import StatisticBox from '@/components/StatisticBox';
import { Skeleton } from '@/components/ui/skeleton';
import { useLedgerQuery } from '@/states/ledger';
import { cn } from '@/lib/utils';
import Amount from '../components/Amount';
import PayeeNarration from '../components/basic/PayeeNarration';
import { intervalForRange, intervalStride, useGraphRows } from '@/components/layout/chart-utils';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { ledgerDate } from '@/components/query/explore-link';
import { BalanceTrendChart, CashFlowChart } from '../components/ReportGraph';
import Section from '../components/Section';

type AccountTypeStatistic = OpReturnType<operations['get_statistic_rank_detail_by_account_type']>['data'];
type TopTransaction = AccountTypeStatistic['top_transactions'][number];

export default function Report() {
  const { t } = useTranslation();

  // Default to the month of the latest activity (the current month for a ledger that is kept up to date).
  const recent = useRecentJournals(1);
  const latest = recent.records[0];
  const defaultRange = useMemo(() => monthOf(activityAnchor(latest).anchor), [latest]);
  const [picked, setPicked] = useState<DateRangeValue | undefined>(undefined);
  const range = picked ?? defaultRange;
  const ready = picked !== undefined || !recent.loading;
  // the days picked, as ledger dates: the report covers them whatever the browser's timezone
  const params = { from: ledgerDate(range.from), to: ledgerDate(range.to) };
  const deps = [ready, params.from, params.to];
  const interval = intervalForRange(range.from, range.to);

  const summary = useLedgerQuery(() => (ready ? retrieveStatisticSummary(params) : undefined), deps);
  const graph = useLedgerQuery(() => (ready ? retrieveStatisticGraph({ ...params, interval }) : undefined), deps);
  const income = useLedgerQuery(() => (ready ? retrieveStatisticByAccountType({ ...params, account_type: 'Income' }) : undefined), deps);
  const expenses = useLedgerQuery(() => (ready ? retrieveStatisticByAccountType({ ...params, account_type: 'Expenses' }) : undefined), deps);
  const { rows, commodity } = useGraphRows(graph.value, interval);
  // the operating currency, which the report's queries value everything in
  const currency = summary.value?.balance.calculated.commodity;
  /** "Open query" for the built-in query behind a card, once the values it ran with are known. */
  const openQuery = (name: string, values: Record<string, string>) =>
    currency ? <OpenInExplore iconOnly name={name} params={{ ...values, currency }} /> : undefined;
  const rangeOnly = { from: params.from, to: params.to };

  const latestPreset: DateRangePreset[] =
    latest && activityAnchor(latest).stale
      ? [{ key: 'latest_activity', label: t('ledger.range.latest_activity'), range: monthOf(new Date(latest.datetime)) }]
      : [];

  const summaryLoading = !summary.value && !summary.error;
  const data = summary.value;
  const graphLoading = !graph.value && !graph.error;
  // A new range keeps showing the previous numbers until every request is back: dim them and say so.
  const refreshing = [summary, graph, income, expenses].some((state) => state.refreshing);

  return (
    <PageShell>
      <PageHeader
        title={t('NAV_REPORT')}
        description={refreshing ? <RefreshingLabel /> : t('ledger.report.description')}
        actions={<DateRangePicker value={range} onChange={setPicked} extraPresets={latestPreset} />}
      />

      <div aria-busy={refreshing} className={cn('flex flex-col gap-4 transition-opacity md:gap-6', refreshing && 'opacity-60')}>
        {summary.error ? (
          <EmptyState icon={CircleAlert} title={t('ledger.common.load_failed')} description={String(summary.error)} />
        ) : (
          <div className="grid grid-cols-2 gap-3 md:gap-4 lg:grid-cols-4">
            <StatisticBox
              text="ASSET_BALANCE"
              icon={Landmark}
              loading={summaryLoading}
              amount={data?.balance.calculated.number ?? '0'}
              currency={data?.balance.calculated.commodity ?? ''}
              hint={t('ledger.report.at_end')}
              action={openQuery('report.net_worth', { to: params.to })}
            />
            <StatisticBox
              text="ledger.chart.income"
              icon={ArrowDownLeft}
              loading={summaryLoading}
              amount={data?.income.calculated.number ?? '0'}
              currency={data?.income.calculated.commodity ?? ''}
              negative
              hint={t('ledger.report.in_period')}
              action={openQuery('report.flows', rangeOnly)}
            />
            <StatisticBox
              text="ledger.chart.expenses"
              icon={ArrowUpRight}
              loading={summaryLoading}
              amount={data?.expense.calculated.number ?? '0'}
              currency={data?.expense.calculated.commodity ?? ''}
              hint={t('ledger.report.in_period')}
              action={openQuery('report.flows', rangeOnly)}
            />
            <StatisticBox
              text="ledger.report.transaction_count"
              icon={Hash}
              loading={summaryLoading}
              amount={(data?.transaction_number ?? 0).toLocaleString()}
              action={<OpenInExplore iconOnly name="report.transaction_count" params={rangeOnly} />}
              hint={<NetFlow income={data?.income.calculated.number} expense={data?.expense.calculated.number} commodity={data?.income.calculated.commodity} />}
            />
          </div>
        )}

        <div className="grid gap-4 md:gap-6 xl:grid-cols-2">
          <Section
            title={t('ledger.chart.net_worth')}
            description={t(`ledger.report.interval_${interval}`)}
            rightSection={openQuery('report.net_worth_trend', { ...rangeOnly, interval: intervalStride(interval) })}
          >
            {graphLoading ? <Skeleton className="h-56 w-full md:h-64" /> : <BalanceTrendChart rows={rows} commodity={commodity} className="h-56 md:h-64" />}
          </Section>
          <Section
            title={t('ledger.chart.income_expenses')}
            description={t(`ledger.report.interval_${interval}`)}
            rightSection={openQuery('report.changes', { ...rangeOnly, interval: intervalStride(interval) })}
          >
            {graphLoading ? <Skeleton className="h-56 w-full md:h-64" /> : <CashFlowChart rows={rows} commodity={commodity} className="h-56 md:h-64" />}
          </Section>
        </div>

        <div className="grid gap-4 md:gap-6 xl:grid-cols-2">
          <Breakdown
            title={t('ledger.report.expense_breakdown')}
            data={expenses.value}
            total={data?.expense.calculated}
            loading={!expenses.value && !expenses.error}
            action={openQuery('report.account_totals', { ...rangeOnly, type: 'Expenses' })}
          />
          <Breakdown
            title={t('ledger.report.income_breakdown')}
            data={income.value}
            total={data?.income.calculated}
            loading={!income.value && !income.error}
            negative
            action={openQuery('report.account_totals', { ...rangeOnly, type: 'Income' })}
          />
        </div>

        <TopTransactions
          title={t('ledger.report.top_expenses')}
          data={expenses.value}
          loading={!expenses.value && !expenses.error}
          action={openQuery('report.top_postings', { ...rangeOnly, type: 'Expenses' })}
        />
        <TopTransactions
          title={t('ledger.report.top_incomes')}
          data={income.value}
          loading={!income.value && !income.error}
          negative
          action={openQuery('report.top_postings', { ...rangeOnly, type: 'Income' })}
        />
      </div>
    </PageShell>
  );
}

function NetFlow({ income, expense, commodity }: { income?: string; expense?: string; commodity?: string }) {
  const { t } = useTranslation();
  if (income === undefined || expense === undefined || !commodity) return null;
  // Income is stored negative: net = -(income) - expense.
  const net = new BigNumber(income).negated().minus(expense);
  return (
    <span className="inline-flex items-baseline gap-1">
      {t('ledger.report.net')} <Amount tone signed amount={net} currency={commodity} />
    </span>
  );
}

/**
 * Totals per account as horizontal bars (largest first), sized relative to the largest account, each with its share of the
 * type's `total`. Bars use the cash-flow chart colours: income (`negative`, stored as negative numbers) chart-1, expenses
 * chart-2.
 */
function Breakdown({
  title,
  data,
  total: typeTotal,
  loading,
  negative,
  action,
}: {
  title: string;
  data?: AccountTypeStatistic;
  /** The type's total in the range: the summary's figure, which the server values as it values the accounts' rows. */
  total?: { number: string; commodity: string };
  loading: boolean;
  negative?: boolean;
  action?: ReactNode;
}) {
  const { t } = useTranslation();
  const items = useMemo(() => {
    const rows = (data?.detail ?? []).map((it) => ({
      account: it.account,
      commodity: it.amount.calculated.commodity,
      value: new BigNumber(it.amount.calculated.number).multipliedBy(negative ? -1 : 1),
    }));
    return rows.filter((it) => !it.value.isZero()).sort((a, b) => b.value.comparedTo(a.value) ?? 0);
  }, [data, negative]);
  const total = typeTotal ? new BigNumber(typeTotal.number).multipliedBy(negative ? -1 : 1) : undefined;
  const largest = items[0]?.value.abs() ?? new BigNumber(1);

  return (
    <Section
      title={title}
      rightSection={
        <>
          {typeTotal && total?.isZero() === false && <Amount className="text-sm font-semibold" amount={total} currency={typeTotal.commodity} />}
          {action}
        </>
      }
    >
      {loading ? (
        <div className="flex flex-col gap-3">
          {[80, 60, 45, 30].map((width) => (
            <Skeleton key={width} className="h-8" style={{ width: `${width}%` }} />
          ))}
        </div>
      ) : items.length === 0 ? (
        <EmptyState icon={ReceiptText} title={t('ledger.report.no_entries')} className="py-8" />
      ) : (
        <ul className="flex flex-col gap-3">
          {items.map((it) => {
            // no share until the summary is back
            const share = total === undefined ? null : total.isZero() ? 0 : it.value.dividedBy(total).multipliedBy(100).toNumber();
            return (
              <li key={it.account} className="flex flex-col gap-1">
                <div className="flex items-baseline justify-between gap-3 text-sm">
                  <span className="min-w-0 truncate" title={it.account}>
                    {it.account}
                  </span>
                  <span className="flex shrink-0 items-baseline gap-2">
                    <Amount amount={it.value} currency={it.commodity} />
                    <span className="w-10 text-right text-xs text-muted-foreground tabular-nums">{share === null ? '' : `${Math.round(share)}%`}</span>
                  </span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    className={cn('h-full rounded-full', negative ? 'bg-chart-1' : 'bg-chart-2')}
                    style={{ width: `${Math.max(2, it.value.abs().dividedBy(largest).multipliedBy(100).toNumber())}%` }}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </Section>
  );
}

function TopTransactions({
  title,
  data,
  loading,
  negative,
  action,
}: {
  title: string;
  data?: AccountTypeStatistic;
  loading: boolean;
  negative?: boolean;
  action?: ReactNode;
}) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-medium">{title}</h2>
        {action}
      </div>
      <ResponsiveList<TopTransaction>
        items={data?.top_transactions ?? []}
        loading={loading}
        getKey={(item, index) => `${item.trx_id}-${item.account}-${index}`}
        empty={<EmptyState icon={ReceiptText} title={t('ledger.report.no_entries')} className="py-8" />}
        columns={[
          {
            key: 'date',
            header: t('ledger.account.col_date'),
            className: 'w-32 pl-4 text-muted-foreground tabular-nums',
            cell: (item) => fmt.date(new Date(item.datetime)),
          },
          {
            key: 'account',
            header: t('ledger.report.col_account'),
            // Sized by its content (the description column takes the rest); only very long names truncate.
            cell: (item) => (
              <span className="block max-w-md truncate" title={item.account}>
                {item.account}
              </span>
            ),
          },
          {
            key: 'payee',
            header: t('ledger.journals.col_description'),
            className: 'max-w-0 w-full',
            cell: (item) => <PayeeNarration payee={item.payee} narration={item.narration} />,
          },
          {
            key: 'amount',
            header: t('ledger.journals.col_amount'),
            className: 'pr-4 text-right font-medium',
            cell: (item) => <Amount amount={item.inferred_unit.number} negative={negative} currency={item.inferred_unit.commodity} />,
          },
        ]}
        renderCard={(item) => (
          <div className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 flex-col gap-0.5">
              <span className="truncate text-sm font-medium">{item.narration || item.payee || '—'}</span>
              <span className="truncate text-xs text-muted-foreground">{item.account}</span>
              <span className="text-xs text-muted-foreground tabular-nums">{fmt.date(new Date(item.datetime))}</span>
            </div>
            <Amount className="shrink-0 text-sm font-semibold" amount={item.inferred_unit.number} negative={negative} currency={item.inferred_unit.commodity} />
          </div>
        )}
      />
    </section>
  );
}
