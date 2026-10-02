import { useAtomValue, useSetAtom } from 'jotai';
import { ArrowRight, ChartColumn, CircleAlert, NotebookText } from 'lucide-react';
import { type ReactNode, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import { useAsync } from 'react-use';
import { retrieveStatisticGraph } from '@/api/requests';
import MobileViewJournalLine from '@/components/journalLines/mobileView/MobileViewJournalLine';
import { EmptyState, PageHeader, PageShell, useIsMobile } from '@/components/layout';
import { formatRange, useDateFormat } from '@/components/layout/use-date-format';
import { activityAnchor, trailingMonth, useRecentJournals } from '@/components/layout/use-ledger-activity';
import { TransactionEditModal } from '@/components/modals/TransactionEditModal';
import { TransactionPreviewModal } from '@/components/modals/TransactionPreviewModal';
import { JournalCardsSkeleton } from '@/components/skeletons/journalListSkeleton';
import { Skeleton } from '@/components/ui/skeleton';
import { buttonVariants } from '@/components/ui/button';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { DASHBOARD_LINK } from '@/layout/nav-links';
import ErrorBox from '../components/ErrorBox';
import { useGraphRows } from '@/components/layout/chart-utils';
import { BalanceTrendChart, CashFlowChart } from '../components/ReportGraph';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import StatisticBox from '../components/StatisticBox';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { errorCountAtom } from '../states/errors';

const LINK_BUTTON = buttonVariants({ variant: 'ghost', size: 'sm', className: 'h-10 md:h-7' });

function Home() {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const isMobile = useIsMobile();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const errorCount = useAtomValue(errorCountAtom);
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

  const graph = useAsync(async () => {
    if (!ready) return undefined;
    const res = await retrieveStatisticGraph({ from: range.from.toISOString(), to: range.to.toISOString(), interval: 'Day' });
    return res.data.data;
  }, [ready, range.from.getTime(), range.to.getTime()]);
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
      <Skeleton className="h-48 w-full md:h-60" />
    ) : graph.error ? (
      <EmptyState icon={CircleAlert} title={t('ledger.common.load_failed')} className="h-48 md:h-60" />
    ) : (
      chart
    );

  return (
    <PageShell>
      <TransactionPreviewModal />
      <TransactionEditModal />
      <PageHeader
        title={t('NAV_DASHBOARD')}
        description={description ?? <span className="inline-block h-4 w-56 animate-pulse rounded-md bg-muted align-middle" />}
        actions={
          isMobile ? undefined : (
            <Link to="/report" className={buttonVariants({ variant: 'outline' })}>
              <ChartColumn data-icon="inline-start" />
              {t('ledger.home.open_report')}
            </Link>
          )
        }
      />

      {ready ? (
        <StatisticBar from={range.from} to={range.to} periodLabel={t('ledger.home.last_30_days')} />
      ) : (
        <div className="grid grid-cols-2 gap-3 md:gap-4 lg:grid-cols-4">
          {['ASSET_BALANCE', 'LIABILITY', 'ledger.chart.income', 'ledger.chart.expenses'].map((key) => (
            <StatisticBox key={key} text={key} amount="0" loading hint=" " />
          ))}
        </div>
      )}

      <div className="grid gap-4 md:gap-6 lg:grid-cols-3">
        <div className="flex min-w-0 flex-col gap-4 md:gap-6 lg:col-span-2">
          <Section title={t('ledger.chart.net_worth')} description={t('ledger.home.net_worth_description')}>
            {chartBody(<BalanceTrendChart rows={rows} commodity={commodity} className="h-48 md:h-60" />)}
          </Section>
          <Section title={t('ledger.chart.income_expenses')} description={t('ledger.home.cash_flow_description')}>
            {chartBody(<CashFlowChart rows={rows} commodity={commodity} className="h-48 md:h-60" />)}
          </Section>
        </div>

        <div className="flex min-w-0 flex-col gap-4 md:gap-6">
          <Section
            title={t('ledger.home.recent_activity')}
            noPadding
            rightSection={
              <Link to="/journals" className={LINK_BUTTON}>
                {t('ledger.home.view_all')}
                <ArrowRight data-icon="inline-end" />
              </Link>
            }
          >
            {recent.loading ? (
              <div className="p-3">
                <JournalCardsSkeleton groups={1} rows={4} />
              </div>
            ) : recent.records.length === 0 ? (
              <EmptyState icon={NotebookText} title={t('ledger.journals.empty_title')} description={t('ledger.journals.empty_description')} className="m-3" />
            ) : (
              <div className="divide-y">
                {recent.records.map((journal) => (
                  <div key={journal.id}>
                    <MobileViewJournalLine data={journal} showDate />
                  </div>
                ))}
              </div>
            )}
          </Section>

          <Section title={t('ledger.home.errors', { count: errorCount })}>
            <ErrorBox />
          </Section>
        </div>
      </div>
    </PageShell>
  );
}

export default Home;
