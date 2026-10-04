import { useAtomValue, useSetAtom } from 'jotai';
import { groupBy, sortBy } from 'lodash-es';
import { PiggyBank, RotateCw } from 'lucide-react';
import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router';
import { useAsyncRetry } from 'react-use';
import { retrieveBudgets } from '@/api/requests';
import Amount from '@/components/Amount';
import BudgetCategory from '@/components/budget/BudgetCategory';
import { budgetUsage, monthFromSearchParams, monthSearchParams, sumByCommodity, usageProgressClass } from '@/components/budget/budget-utils';
import { MonthSwitcher } from '@/components/budget/MonthSwitcher';
import { EmptyState, LoadFailedState, PageHeader, PageShell } from '@/components/layout';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { KeyFigure, KeyFigures } from '@/components/layout/KeyFigures';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Progress } from '@/components/ui/progress';
import { Skeleton } from '@/components/ui/skeleton';
import { Switch } from '@/components/ui/switch';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { BUDGETS_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

const UNCATEGORIZED = '__ZHANG_UNCATEGORIZED__';

function AmountList({ amounts }: { amounts: ReturnType<typeof sumByCommodity> }) {
  if (amounts.length === 0) return <span>—</span>;
  return (
    <span className="flex flex-col">
      {amounts.map((amount) => (
        <Amount key={amount.commodity} amount={amount.number} currency={amount.commodity} />
      ))}
    </span>
  );
}

function BudgetsSkeleton() {
  return (
    <div className="flex flex-col gap-3">
      <Skeleton className="h-6 w-40" />
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {Array.from({ length: 6 }, (_, index) => (
          <Skeleton key={index} className="h-32 rounded-xl" />
        ))}
      </div>
    </div>
  );
}

export default function Budgets() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const [searchParams, setSearchParams] = useSearchParams();
  const date = useMemo(() => monthFromSearchParams(searchParams), [searchParams]);
  const setDate = (next: Date) => setSearchParams(monthSearchParams(next), { replace: true });
  const [hideZeroAssignBudget, setHideZeroAssignBudget] = useLocalStorage({ key: 'hideZeroAssignBudget', defaultValue: false });
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('NAV_BUDGETS')} - ${ledgerTitle}`);

  useEffect(() => {
    // The month lives in the page's own switcher; a "Budget > Oct 2026" crumb would make the mobile back button point at this page.
    setBreadcrumb([BUDGETS_LINK]);
  }, [setBreadcrumb]);

  const {
    loading,
    error,
    value: budgets,
    retry,
  } = useAsyncRetry(async () => {
    const res = await retrieveBudgets({ year: date.getFullYear(), month: date.getMonth() + 1 });
    return res.data.data;
  }, [date.getFullYear(), date.getMonth()]);

  const visibleBudgets = useMemo(
    () => (budgets ?? []).filter((budget) => !hideZeroAssignBudget || Number(budget.assigned_amount.number) !== 0),
    [budgets, hideZeroAssignBudget],
  );
  const categories = useMemo(
    () => sortBy(Object.entries(groupBy(visibleBudgets, (budget) => budget.category ?? UNCATEGORIZED)), ([name]) => (name === UNCATEGORIZED ? '￿' : name)),
    [visibleBudgets],
  );

  const assigned = sumByCommodity(visibleBudgets.map((budget) => budget.assigned_amount));
  const activity = sumByCommodity(visibleBudgets.map((budget) => budget.activity_amount));
  const available = sumByCommodity(visibleBudgets.map((budget) => budget.available_amount));
  const usage = budgetUsage(activity[0]?.number ?? '0', assigned[0]?.number ?? '0');
  const firstLoad = loading && budgets === undefined;

  return (
    <PageShell>
      <PageHeader
        title={t('NAV_BUDGETS')}
        description={t('budgets.description')}
        actions={
          <>
            <MonthSwitcher date={date} onChange={setDate} />
            <OpenInExplore name="budgets.month" params={{ month: date }} iconOnly className="size-10 md:size-8" />
            <Button variant="outline" size="icon" className="size-10 md:size-8" aria-label={t('REFRESH')} onClick={retry} disabled={loading}>
              <RotateCw className={cn(loading && 'animate-spin')} />
            </Button>
          </>
        }
      >
        <Label className="flex min-h-10 w-fit cursor-pointer items-center gap-3 text-sm font-normal text-muted-foreground md:min-h-0">
          <Switch checked={hideZeroAssignBudget} onCheckedChange={(checked) => setHideZeroAssignBudget(checked)} />
          {t('budgets.hide_zero_assigned')}
        </Label>
      </PageHeader>

      {error ? (
        <LoadFailedState description={error.message} onRetry={retry} />
      ) : firstLoad ? (
        <BudgetsSkeleton />
      ) : visibleBudgets.length === 0 ? (
        <EmptyState
          icon={PiggyBank}
          title={(budgets ?? []).length === 0 ? t('budgets.empty_title') : t('budgets.all_hidden_title')}
          description={(budgets ?? []).length === 0 ? t('budgets.empty_description') : t('budgets.all_hidden_description')}
          action={
            (budgets ?? []).length > 0 && (
              <Button variant="outline" className="h-10 md:h-8" onClick={() => setHideZeroAssignBudget(false)}>
                {t('budgets.show_all')}
              </Button>
            )
          }
        />
      ) : (
        <>
          <KeyFigures>
            <KeyFigure label={t('budgets.assigned')} value={<AmountList amounts={assigned} />} />
            <KeyFigure label={t('budgets.activity')} value={<AmountList amounts={activity} />} />
            <KeyFigure label={t('budgets.available')} value={<AmountList amounts={available} />} />
            <KeyFigure
              label={t('budgets.used')}
              value={<span className={cn(usage.over && 'text-destructive')}>{usage.label}</span>}
              hint={t('budgets.budget_count', { count: visibleBudgets.length })}
            >
              <Progress value={usage.percent} aria-label={t('budgets.used')} className={cn('mt-1', usageProgressClass(usage.over))} />
            </KeyFigure>
          </KeyFigures>
          <div className={cn('flex flex-col gap-6 transition-opacity', loading && 'opacity-60')}>
            {categories.map(([name, items]) => (
              <BudgetCategory
                key={`${name}-${date.getFullYear()}-${date.getMonth()}`}
                name={name}
                label={name === UNCATEGORIZED ? t('budgets.uncategorized') : name}
                items={items}
                search={`?year=${date.getFullYear()}&month=${date.getMonth() + 1}`}
              />
            ))}
          </div>
        </>
      )}
    </PageShell>
  );
}
