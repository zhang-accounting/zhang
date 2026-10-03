import { useAtomValue, useSetAtom } from 'jotai';
import { ChartColumn, CircleAlert, NotebookText } from 'lucide-react';
import { type ReactNode, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import { useAsync } from 'react-use';
import { retrieveStatisticGraph } from '@/api/requests';
import { MonthBudgetsCard } from '@/components/budget/MonthBudgetsCard';
import { JournalRow } from '@/components/journalLines/JournalRow';
import { EmptyState, PageHeader, PageShell, useIsMobile } from '@/components/layout';
import { intervalStride, useGraphRows } from '@/components/layout/chart-utils';
import { formatRange, useDateFormat } from '@/components/layout/use-date-format';
import { activityAnchor, trailingMonth, useRecentJournals } from '@/components/layout/use-ledger-activity';
import { TransactionEditModal } from '@/components/modals/TransactionEditModal';
import { TransactionPreviewModal } from '@/components/modals/TransactionPreviewModal';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { ledgerDate } from '@/components/query/explore-link';
import { JournalRowsSkeleton } from '@/components/skeletons/journalListSkeleton';
import { buttonVariants } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Skeleton } from '@/components/ui/skeleton';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { DASHBOARD_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import ErrorBox from '../components/ErrorBox';
import { BalanceTrendChart, CashFlowChart } from '../components/ReportGraph';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import StatisticBox from '../components/StatisticBox';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { errorCountAtom } from '../states/errors';

const CHART_HEIGHT = 'h-48 md:h-56';
const ERRORS_BUTTON_CLASS = cn(
  '-my-2 inline-flex h-10 items-center rounded-sm text-negative outline-none md:h-6',
  'hover:underline focus-visible:ring-2 focus-visible:ring-ring',
);
/** Charts grow with their grid row (the budget / recent-activity card next to them can be taller). */
const CHART_FILL = 'h-auto min-h-48 flex-1 md:min-h-56';

/** Footer line of the recent-activity card: the otter + "healthy", or the error count opening the error list. */
function LedgerHealth() {
  const { t } = useTranslation();
  const errorCount = useAtomValue(errorCountAtom);
  const [open, setOpen] = useState(false);

  return (
    <div className="flex items-center gap-2.5 border-t px-4 py-2.5 text-[13px] text-foreground-2">
      <img src="/otter-192.png" alt="" className="size-6 shrink-0 rounded-md" />
      {errorCount === 0 ? (
        <span>{t('LEDGER_IS_HEALTHY')}</span>
      ) : (
        <>
          <button type="button" className={ERRORS_BUTTON_CLASS} onClick={() => setOpen(true)}>
            {t('ledger.home.errors', { count: errorCount })}
          </button>
          <Dialog open={open} onOpenChange={setOpen}>
            <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-xl">
              <DialogHeader>
                <DialogTitle>{t('ledger.home.errors', { count: errorCount })}</DialogTitle>
                <DialogDescription>{t('ledger.home.errors_description')}</DialogDescription>
              </DialogHeader>
              <ErrorBox />
            </DialogContent>
          </Dialog>
        </>
      )}
    </div>
  );
}

function Home() {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const isMobile = useIsMobile();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('NAV_DASHBOARD')} - ${ledgerTitle}`);

  useEffect(() => {
    setBreadcrumb([DASHBOARD_LINK]);
  }, [setBreadcrumb]);

  const recent = useRecentJournals(6);
  const latest = recent.records[0];
  const ready = !recent.loading;
  const { range, stale } = useMemo(() => {
    const { anchor, stale } = activityAnchor(latest);
    return { range: trailingMonth(anchor), stale };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latest?.datetime]);

  // the days of the range, as ledger dates
  const dates = { from: ledgerDate(range.from), to: ledgerDate(range.to) };
  const graph = useAsync(async () => {
    if (!ready) return undefined;
    const res = await retrieveStatisticGraph({ ...dates, interval: 'Day' });
    return res.data.data;
  }, [ready, dates.from, dates.to]);
  const { rows, commodity } = useGraphRows(graph.value, 'Day');
  const graphLoading = !ready || graph.loading || (!graph.value && !graph.error);

  const rangeLabel = formatRange(fmt, range.from, range.to);
  const description = !ready
    ? undefined
    : stale
      ? t('ledger.home.description_stale', { range: rangeLabel })
      : t('ledger.home.description', { range: rangeLabel });

  const chartBody = (chart: ReactNode) =>
    graphLoading ? (
      <Skeleton className={cn(CHART_HEIGHT, 'w-full')} />
    ) : graph.error ? (
      <EmptyState icon={CircleAlert} title={t('ledger.common.load_failed')} className={CHART_HEIGHT} />
    ) : (
      chart
    );
  // Chart hints sit right of the title from sm up and under it on phones (the title would otherwise be truncated).
  const chartHint = (text: string) => <span className="hidden text-xs text-muted-foreground sm:inline">{text}</span>;
  const chartHintMobile = (text: string) => <span className="sm:hidden">{text}</span>;
  /** "Open query" for the built-in query behind a chart, once its currency is known. */
  const openQuery = (name: string) =>
    commodity ? <OpenInExplore iconOnly name={name} params={{ ...dates, interval: intervalStride('Day'), currency: commodity }} /> : undefined;

  return (
    <PageShell className="gap-3 md:gap-3">
      <TransactionPreviewModal />
      <TransactionEditModal />
      <PageHeader
        className="mb-1"
        title={t('NAV_DASHBOARD')}
        description={description ?? <span className="inline-block h-4 w-56 animate-pulse rounded-md bg-muted align-middle" />}
        actions={
          isMobile ? undefined : (
            <Link to="/report" className={cn(buttonVariants({ variant: 'outline' }), 'bg-card')}>
              <ChartColumn data-icon="inline-start" />
              {t('ledger.home.open_report')}
            </Link>
          )
        }
      />

      {ready ? (
        <StatisticBar from={range.from} to={range.to} periodLabel={t('ledger.home.last_30_days')} />
      ) : (
        <div className="grid grid-cols-2 gap-2.5 md:gap-3 lg:grid-cols-4">
          {['ASSET_BALANCE', 'LIABILITY', 'ledger.chart.income', 'ledger.chart.expenses'].map((key) => (
            <StatisticBox key={key} text={key} amount="0" loading hint=" " />
          ))}
        </div>
      )}

      <div className="grid gap-3 lg:grid-cols-3">
        <Section
          className="min-w-0 lg:col-span-2"
          contentClassName="flex flex-1 flex-col"
          title={t('ledger.chart.net_worth')}
          description={chartHintMobile(t('ledger.home.net_worth_description'))}
          rightSection={
            <>
              {chartHint(t('ledger.home.net_worth_description'))}
              {openQuery('report.net_worth_trend')}
            </>
          }
        >
          {chartBody(<BalanceTrendChart rows={rows} commodity={commodity} className={CHART_FILL} />)}
        </Section>
        {ready ? (
          <MonthBudgetsCard month={range.to} className="min-w-0" />
        ) : (
          <Section className="min-w-0" title={t('ledger.home.month_budgets')}>
            <Skeleton className="h-40 w-full" />
          </Section>
        )}

        <Section
          className="min-w-0 lg:col-span-2"
          contentClassName="flex flex-1 flex-col"
          title={t('ledger.chart.income_expenses')}
          description={chartHintMobile(t('ledger.home.cash_flow_description'))}
          rightSection={
            <>
              {chartHint(t('ledger.home.cash_flow_description'))}
              {openQuery('report.changes')}
            </>
          }
        >
          {chartBody(<CashFlowChart rows={rows} commodity={commodity} className={CHART_FILL} />)}
        </Section>
        <Section
          className="min-w-0"
          contentClassName="flex flex-1 flex-col"
          noPadding
          divider={false}
          title={t('ledger.home.recent_activity')}
          rightSection={
            <Link to="/journals" className="inline-flex h-10 items-center text-xs text-link hover:underline md:h-6">
              {t('ledger.home.view_all')}
            </Link>
          }
        >
          <div className="flex-1 divide-y border-t">
            {recent.loading ? (
              <JournalRowsSkeleton rows={3} />
            ) : recent.records.length === 0 ? (
              <EmptyState icon={NotebookText} title={t('ledger.journals.empty_title')} description={t('ledger.journals.empty_description')} className="m-3" />
            ) : (
              recent.records.slice(0, 4).map((journal) => <JournalRow key={journal.id} data={journal} showDate dense />)
            )}
          </div>
          <LedgerHealth />
        </Section>
      </div>
    </PageShell>
  );
}

export default Home;
