import { Buffer } from 'buffer';
import { ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { BudgetListItem } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { Progress } from '@/components/ui/progress';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { cn } from '@/lib/utils';
import Amount from '../Amount';
import { budgetUsage, sumByCommodity, usageProgressClass } from './budget-utils';
import BudgetLine from './BudgetLine';

interface Props {
  name: string;
  /** Display name; defaults to `name` (the raw category key). */
  label?: string;
  items: BudgetListItem[];
  /** Query string for the budget detail links. */
  search?: string;
}

/** A budget category: collapsible header with the category totals, then a grid of budget cards. */
export default function BudgetCategory(props: Props) {
  const { t } = useTranslation();
  const [isShow, setCollapse] = useLocalStorage({
    key: `budget-category-${Buffer.from(props.name).toString('base64')}-collapse`,
    defaultValue: true,
  });

  const assigned = sumByCommodity(props.items.map((item) => item.assigned_amount));
  const activity = sumByCommodity(props.items.map((item) => item.activity_amount));
  const primaryAssigned = assigned[0];
  const primaryActivity = activity.find((it) => it.commodity === primaryAssigned?.commodity) ?? activity[0];
  const usage = budgetUsage(primaryActivity?.number.toString() ?? '0', primaryAssigned?.number.toString() ?? '0');
  const sortedItems = [...props.items].sort((a, b) => (a.alias ?? a.name).localeCompare(b.alias ?? b.name));

  return (
    <section className="flex flex-col gap-3">
      {/* The heading holds only the toggle (chevron + name); totals and the progress bar sit outside the button. The button's
          ::after stretches over the whole row, so the row stays one big click target. */}
      <div
        className={cn(
          'relative -mx-2 flex min-h-10 items-center gap-3 rounded-lg px-2 py-1 transition-colors hover:bg-muted/50',
          'has-[button:focus-visible]:ring-3 has-[button:focus-visible]:ring-ring/50',
        )}
      >
        <h2 className="min-w-0 text-sm font-semibold">
          <button
            type="button"
            aria-expanded={isShow}
            onClick={() => setCollapse(!isShow)}
            className="flex min-w-0 items-center gap-3 text-left outline-none after:absolute after:inset-0 after:rounded-lg"
          >
            <ChevronRight className={cn('size-4 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none', isShow && 'rotate-90')} />
            <span className="truncate">{props.label ?? props.name}</span>
          </button>
        </h2>
        <Badge variant="secondary" className="tabular-nums">
          {props.items.length}
        </Badge>
        <div className="ml-auto flex min-w-0 items-center gap-3">
          <span className="hidden truncate text-xs text-muted-foreground tabular-nums sm:inline">
            {primaryActivity && <Amount amount={primaryActivity.number} currency={primaryActivity.commodity} />}
            {' / '}
            {primaryAssigned && <Amount amount={primaryAssigned.number} currency={primaryAssigned.commodity} />}
          </span>
          <Progress value={usage.percent} aria-label={t('budgets.used')} className={cn('w-16 md:w-24', usageProgressClass(usage.over))} />
          <span className={cn('w-12 text-right text-xs font-medium tabular-nums', usage.over ? 'text-destructive' : 'text-muted-foreground')}>
            {usage.label}
          </span>
        </div>
      </div>
      {isShow && (
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          {sortedItems.map((item) => (
            <BudgetLine key={item.name} {...item} search={props.search} />
          ))}
        </div>
      )}
    </section>
  );
}
