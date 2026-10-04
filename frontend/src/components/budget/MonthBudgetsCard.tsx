import BigNumber from 'bignumber.js';
import { ArrowUpRight } from 'lucide-react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { useAsync } from 'react-use';
import { retrieveBudgets } from '@/api/requests';
import { BudgetListItem } from '@/api/types';
import Amount from '@/components/Amount';
import { budgetUsage, monthSearchParams, sumByCommodity } from '@/components/budget/budget-utils';
import { useDateFormat } from '@/components/layout/use-date-format';
import Section from '@/components/Section';
import { Skeleton } from '@/components/ui/skeleton';
import { BUDGET_DOCS_URL } from '@/layout/nav-links';
import { cn } from '@/lib/utils';

const MAX_ROWS = 5;

function BudgetRow({ budget }: { budget: BudgetListItem }) {
  const { t } = useTranslation();
  const activity = budget.activity_amount;
  const assigned = budget.assigned_amount;
  const usage = budgetUsage(activity.number, assigned.number);
  const overBy = new BigNumber(activity.number).minus(assigned.number);

  return (
    <li className="flex flex-col gap-1.5 py-2.5">
      <div className="flex items-baseline justify-between gap-3 text-[13px]">
        <span className="min-w-0 truncate" title={budget.name}>
          {budget.alias || budget.name}
        </span>
        <span className="flex shrink-0 items-baseline gap-1 tabular-nums">
          <Amount amount={activity.number} currency={activity.commodity} />
          <span className="text-muted-foreground">
            / <Amount amount={assigned.number} currency={assigned.commodity} />
          </span>
        </span>
      </div>
      <div
        className="h-1.5 overflow-hidden rounded-full bg-track"
        role="progressbar"
        aria-label={budget.alias || budget.name}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(usage.percent)}
      >
        <div className={cn('h-full rounded-full', usage.over ? 'bg-negative' : 'bg-primary')} style={{ width: `${usage.percent}%` }} />
      </div>
      <div className="flex items-baseline justify-between gap-3 text-xs text-muted-foreground tabular-nums">
        {usage.over ? (
          <>
            <span className="text-negative">
              {t('ledger.home.budget_over_by')} <Amount amount={overBy} currency={activity.commodity} />
            </span>
            <span>{usage.label}</span>
          </>
        ) : (
          <>
            <span>{t('ledger.home.budget_used', { percent: usage.label })}</span>
            <span>
              {t('ledger.home.budget_left')} <Amount amount={budget.available_amount.number} currency={budget.available_amount.commodity} />
            </span>
          </>
        )}
      </div>
    </li>
  );
}

/**
 * Overview card: this month's budgets (up to five, most activity first) with `activity / assigned`, a 6px bar (primary fill,
 * negative once over budget) and what is left or overspent; the header shows the month and the total left.
 */
export function MonthBudgetsCard({ month, className }: { month: Date; className?: string }) {
  const { t, i18n } = useTranslation();
  const fmt = useDateFormat();
  const year = month.getFullYear();
  const monthIndex = month.getMonth();
  const { value, loading, error } = useAsync(async () => (await retrieveBudgets({ year, month: monthIndex + 1 })).data.data, [year, monthIndex]);

  const open = useMemo(() => (value ?? []).filter((budget) => !budget.closed), [value]);
  const rows = useMemo(() => [...open].sort((a, b) => Number(b.activity_amount.number) - Number(a.activity_amount.number)).slice(0, MAX_ROWS), [open]);
  const left = sumByCommodity(open.map((budget) => budget.available_amount))[0];
  const monthLabel = year === new Date().getFullYear() ? fmt.format(month, i18n.language.startsWith('zh') ? 'M月' : 'MMM') : fmt.month(month);
  const params = new URLSearchParams(monthSearchParams(month)).toString();

  return (
    <Section
      className={className}
      contentClassName="flex flex-1 flex-col"
      noPadding
      divider={false}
      title={t('ledger.home.month_budgets')}
      rightSection={
        left && (
          <span className="text-xs text-muted-foreground tabular-nums">
            {monthLabel} ·{' '}
            {left.number.isNegative() ? (
              <span className="text-negative">
                {t('ledger.home.budget_over')} <Amount amount={left.number.abs()} currency={left.commodity} />
              </span>
            ) : (
              <>
                {t('ledger.home.budget_left')} <Amount amount={left.number} currency={left.commodity} />
              </>
            )}
          </span>
        )
      }
    >
      <div className="flex min-h-0 flex-1 flex-col px-4">
        {loading ? (
          <div className="flex flex-col gap-4 py-3">
            {[0, 1, 2].map((index) => (
              <div key={index} className="flex flex-col gap-2">
                <Skeleton className="h-3.5 w-full" />
                <Skeleton className="h-1.5 w-full" />
              </div>
            ))}
          </div>
        ) : error ? (
          <p className="py-4 text-[13px] text-muted-foreground">{t('ledger.common.load_failed')}</p>
        ) : rows.length === 0 ? (
          <p className="py-4 text-[13px] text-muted-foreground">
            {t('ledger.home.budget_empty')}{' '}
            <a href={BUDGET_DOCS_URL} target="_blank" rel="noreferrer" className="inline-flex items-center gap-0.5 text-link hover:underline">
              {t('ledger.home.budget_docs')}
              <ArrowUpRight className="size-3" aria-hidden />
            </a>
          </p>
        ) : (
          <ul className="flex flex-col divide-y">
            {rows.map((budget) => (
              <BudgetRow key={budget.name} budget={budget} />
            ))}
          </ul>
        )}
      </div>
      {rows.length > 0 && (
        <div className="mt-auto flex justify-end border-t px-4 py-2">
          <Link to={`/budgets?${params}`} className="inline-flex h-10 items-center text-[13px] text-link hover:underline md:h-6">
            {t('ledger.home.view_all')}
          </Link>
        </div>
      )}
    </Section>
  );
}
