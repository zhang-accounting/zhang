import { useAtomValue, useSetAtom } from 'jotai';
import { ArrowLeft, ListX, TriangleAlert } from 'lucide-react';
import { OpReturnType } from 'openapi-typescript-fetch';
import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useParams, useSearchParams } from 'react-router-dom';
import { useAsync, useAsyncRetry } from 'react-use';
import { retrieveBudgetEvent, retrieveBudgetInfo } from '@/api/requests';
import { operations } from '@/api/schemas';
import Amount from '@/components/Amount';
import PayeeNarration from '@/components/basic/PayeeNarration';
import { budgetUsage, monthFromSearchParams, monthSearchParams, usageProgressClass } from '@/components/budget/budget-utils';
import { MonthSwitcher } from '@/components/budget/MonthSwitcher';
import { EmptyState, PageHeader, PageShell, RefreshingLabel, ResponsiveList, type ResponsiveColumn } from '@/components/layout';
import { KeyFigure, KeyFigures } from '@/components/layout/KeyFigures';
import { useDateFormat } from '@/components/layout/use-date-format';
import { Badge } from '@/components/ui/badge';
import { buttonVariants } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { Progress } from '@/components/ui/progress';
import { Skeleton } from '@/components/ui/skeleton';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { BUDGETS_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

type BudgetEvent = OpReturnType<operations['get_budget_interval_detail']>['data'][number];

function isBudgetEvent(event: BudgetEvent): event is Extract<BudgetEvent, { type: 'BudgetEvent' }> {
  return 'event_type' in event;
}

function toneClass(value: string | undefined) {
  const number = Number(value ?? 0);
  if (number < 0) return 'text-negative';
  if (number > 0) return 'text-positive';
  return undefined;
}

function SingleBudget() {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { budgetName } = useParams();
  const [searchParams, setSearchParams] = useSearchParams();
  const date = useMemo(() => monthFromSearchParams(searchParams), [searchParams]);
  const setDate = (next: Date) => setSearchParams(monthSearchParams(next), { replace: true });
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${budgetName} | ${t('NAV_BUDGETS')} - ${ledgerTitle}`);
  const year = date.getFullYear();
  const month = date.getMonth() + 1;

  useEffect(() => {
    setBreadcrumb([
      { ...BUDGETS_LINK, uri: `/budgets?year=${year}&month=${month}` },
      { label: budgetName ?? '', uri: `/budgets/${budgetName}`, noTranslate: true },
    ]);
  }, [budgetName, year, month, setBreadcrumb]);

  const {
    value: budgetInfo,
    error,
    loading: infoLoading,
  } = useAsync(async () => {
    const res = await retrieveBudgetInfo({ budget_name: budgetName ?? '', year, month });
    return res.data.data;
  }, [budgetName, year, month]);
  const {
    value: events,
    loading: eventsLoading,
    error: eventsError,
  } = useAsyncRetry(async () => {
    const res = await retrieveBudgetEvent({ budget_name: budgetName ?? '', year, month });
    return res.data.data;
  }, [budgetName, year, month]);

  const columns: ResponsiveColumn<BudgetEvent>[] = [
    {
      key: 'date',
      header: t('budgets.date'),
      className: 'w-36 text-muted-foreground tabular-nums',
      cell: (event) => fmt.dayTime(event.timestamp * 1000),
    },
    {
      key: 'activity',
      header: t('budgets.description_column'),
      cell: (event) =>
        isBudgetEvent(event) ? <span>{t(`budgets.event.${event.event_type}`)}</span> : <PayeeNarration payee={event.payee} narration={event.narration} />,
    },
    {
      key: 'account',
      header: t('budgets.account'),
      cell: (event) =>
        !isBudgetEvent(event) && (
          <Badge variant="outline" render={<Link to={`/accounts/${event.account}`} />}>
            {event.account}
          </Badge>
        ),
    },
    {
      key: 'assigned',
      header: t('budgets.assigned'),
      className: 'text-right tabular-nums',
      cell: (event) => isBudgetEvent(event) && <Amount amount={event.amount.number} currency={event.amount.commodity} />,
    },
    {
      key: 'activity_amount',
      header: t('budgets.activity'),
      className: 'text-right tabular-nums',
      cell: (event) => !isBudgetEvent(event) && <Amount amount={event.inferred_unit.number} currency={event.inferred_unit.commodity} />,
    },
  ];

  const renderCard = (event: BudgetEvent) => (
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium">
          {isBudgetEvent(event) ? t(`budgets.event.${event.event_type}`) : [event.payee, event.narration].filter(Boolean).join(' · ') || event.account}
        </div>
        <div className="truncate text-xs text-muted-foreground">
          {fmt.dayTime(event.timestamp * 1000)}
          {!isBudgetEvent(event) && ` · ${event.account}`}
        </div>
      </div>
      <div className="shrink-0 text-right text-sm font-medium tabular-nums">
        {isBudgetEvent(event) ? (
          <>
            <Amount amount={event.amount.number} currency={event.amount.commodity} />
            <div className="text-xs font-normal text-muted-foreground">{t('budgets.assigned')}</div>
          </>
        ) : (
          <>
            <Amount amount={event.inferred_unit.number} currency={event.inferred_unit.commodity} />
            <div className="text-xs font-normal text-muted-foreground">{t('budgets.activity')}</div>
          </>
        )}
      </div>
    </div>
  );

  if (error) {
    return (
      <PageShell>
        <PageHeader title={budgetName} />
        <EmptyState
          icon={TriangleAlert}
          title={t('budgets.not_found_title')}
          description={t('budgets.not_found_description', { name: budgetName })}
          action={
            <Link to="/budgets" className={cn(buttonVariants({ variant: 'outline' }), 'h-10 md:h-8')}>
              <ArrowLeft />
              {t('budgets.back_to_budgets')}
            </Link>
          }
        />
      </PageShell>
    );
  }

  const usage = budgetInfo ? budgetUsage(budgetInfo.activity_amount.number, budgetInfo.assigned_amount.number) : undefined;
  const firstLoad = infoLoading && budgetInfo === undefined;
  // Switching month keeps the previous month's numbers on screen until the requests are back: dim them and say so.
  const refreshing = (infoLoading && budgetInfo !== undefined) || (eventsLoading && events !== undefined);

  return (
    <PageShell>
      <PageHeader
        title={firstLoad ? <Skeleton className="h-7 w-48" /> : (budgetInfo?.alias ?? budgetInfo?.name ?? budgetName)}
        description={refreshing ? <RefreshingLabel /> : t('budgets.detail_description', { month: fmt.month(date) })}
        actions={
          <>
            <MonthSwitcher date={date} onChange={setDate} />
            <OpenInExplore name="budgets.budget_month" params={{ name: budgetName ?? '', month: date }} iconOnly className="size-10 md:size-8" />
          </>
        }
      >
        {budgetInfo && (budgetInfo.alias || budgetInfo.category || budgetInfo.closed) && (
          <div className="flex flex-wrap items-center gap-2">
            {budgetInfo.alias && <Badge variant="outline">{budgetInfo.name}</Badge>}
            {budgetInfo.category && <Badge variant="secondary">{budgetInfo.category}</Badge>}
            {budgetInfo.closed && <Badge variant="destructive">{t('budgets.closed')}</Badge>}
          </div>
        )}
      </PageHeader>

      <KeyFigures aria-busy={refreshing} className={cn('transition-opacity', refreshing && 'opacity-60')}>
        <KeyFigure
          loading={firstLoad}
          label={t('budgets.assigned')}
          value={budgetInfo && <Amount amount={budgetInfo.assigned_amount.number} currency={budgetInfo.assigned_amount.commodity} />}
        />
        <KeyFigure
          loading={firstLoad}
          label={t('budgets.activity')}
          value={budgetInfo && <Amount amount={budgetInfo.activity_amount.number} currency={budgetInfo.activity_amount.commodity} />}
        />
        <KeyFigure
          loading={firstLoad}
          label={t('budgets.available')}
          valueClassName={toneClass(budgetInfo?.available_amount.number)}
          value={budgetInfo && <Amount amount={budgetInfo.available_amount.number} currency={budgetInfo.available_amount.commodity} />}
        />
        <KeyFigure loading={firstLoad} label={t('budgets.used')} value={<span className={cn(usage?.over && 'text-destructive')}>{usage?.label}</span>}>
          {usage && <Progress value={usage.percent} aria-label={t('budgets.used')} className={cn('mt-1', usageProgressClass(usage.over))} />}
        </KeyFigure>
      </KeyFigures>

      <Card size="sm">
        <CardHeader>
          <CardTitle>{t('budgets.related_accounts')}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          {firstLoad ? (
            <Skeleton className="h-5 w-56" />
          ) : (budgetInfo?.related_accounts ?? []).length === 0 ? (
            <span className="text-sm text-muted-foreground">{t('budgets.no_related_accounts')}</span>
          ) : (
            budgetInfo?.related_accounts.map((account) => (
              <Badge key={account} variant="outline" className="h-10 max-w-full px-3 md:h-6 md:px-2.5" render={<Link to={`/accounts/${account}`} />}>
                <span className="truncate">{account}</span>
              </Badge>
            ))
          )}
        </CardContent>
      </Card>

      <section aria-busy={refreshing} className={cn('flex flex-col gap-3 transition-opacity', refreshing && 'opacity-60')}>
        <div className="flex items-center justify-between gap-2">
          <h2 className="text-sm font-semibold">{t('budgets.activity_in', { month: fmt.month(date) })}</h2>
          {budgetInfo && (
            <OpenInExplore name="budgets.postings" params={{ accounts: budgetInfo.related_accounts, month: date, name: budgetInfo.name }} iconOnly />
          )}
        </div>
        {eventsError ? (
          <EmptyState icon={TriangleAlert} title={t('page_state.load_failed')} description={eventsError.message} />
        ) : (
          <ResponsiveList
            items={events ?? []}
            loading={eventsLoading && events === undefined}
            getKey={(event, index) => `${event.timestamp}-${index}`}
            columns={columns}
            renderCard={renderCard}
            empty={<EmptyState icon={ListX} title={t('budgets.no_activity_title')} description={t('budgets.no_activity_description')} />}
          />
        )}
      </section>
    </PageShell>
  );
}

export default SingleBudget;
