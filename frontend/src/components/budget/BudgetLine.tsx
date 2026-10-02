import { Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { BudgetListItem } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { Progress } from '@/components/ui/progress';
import { cn } from '@/lib/utils';
import Amount from '../Amount';
import { budgetUsage, usageProgressClass } from './budget-utils';

interface Props extends BudgetListItem {
  /** Query string carried to the detail page (e.g. `?year=2026&month=10`). */
  search?: string;
}

/** One budget as a tappable card: name, usage bar and assigned / activity / available figures. */
export default function BudgetLine(props: Props) {
  const { t } = useTranslation();
  const usage = budgetUsage(props.activity_amount.number, props.assigned_amount.number);
  const available = Number(props.available_amount.number);

  return (
    <Link
      to={`/budgets/${encodeURIComponent(props.name)}${props.search ?? ''}`}
      className={cn(
        'group flex min-w-0 flex-col gap-3 rounded-xl border bg-card p-4 transition-colors outline-none',
        'hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 active:bg-muted',
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate font-medium">{props.alias ?? props.name}</div>
          {props.alias && <div className="truncate text-xs text-muted-foreground">{props.name}</div>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {props.closed && <Badge variant="secondary">{t('budgets.closed')}</Badge>}
          <span className={cn('text-xs font-medium tabular-nums', usage.over ? 'text-destructive' : 'text-muted-foreground')}>{usage.label}</span>
        </div>
      </div>
      <Progress value={usage.percent} aria-label={t('budgets.used')} className={usageProgressClass(usage.over)} />
      <dl className="grid grid-cols-3 gap-2 text-xs">
        <div className="min-w-0">
          <dt className="text-muted-foreground">{t('budgets.assigned')}</dt>
          <dd className="truncate font-medium tabular-nums">
            <Amount amount={props.assigned_amount.number} currency={props.assigned_amount.commodity} />
          </dd>
        </div>
        <div className="min-w-0">
          <dt className="text-muted-foreground">{t('budgets.activity')}</dt>
          <dd className="truncate font-medium tabular-nums">
            <Amount amount={props.activity_amount.number} currency={props.activity_amount.commodity} />
          </dd>
        </div>
        <div className="min-w-0 text-right">
          <dt className="text-muted-foreground">{t('budgets.available')}</dt>
          <dd
            className={cn(
              'truncate font-medium tabular-nums',
              available < 0 ? 'text-destructive' : available > 0 ? 'text-emerald-600 dark:text-emerald-400' : undefined,
            )}
          >
            <Amount amount={props.available_amount.number} currency={props.available_amount.commodity} />
          </dd>
        </div>
      </dl>
    </Link>
  );
}
