import { addMonths, isSameMonth } from 'date-fns';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { formatMonth } from './budget-utils';

interface Props {
  date: Date;
  onChange: (date: Date) => void;
  className?: string;
}

/** Previous / current / next month control for budget intervals. Future months are not reachable. */
export function MonthSwitcher({ date, onChange, className }: Props) {
  const { t, i18n } = useTranslation();
  const isCurrentMonth = isSameMonth(date, new Date());

  return (
    <div className={cn('flex items-center gap-1', className)}>
      <div className="flex items-center rounded-lg border bg-background">
        <Button
          variant="ghost"
          size="icon"
          className="size-10 rounded-r-none md:size-8"
          aria-label={t('budgets.previous_month')}
          onClick={() => onChange(addMonths(date, -1))}
        >
          <ChevronLeft />
        </Button>
        <span className="min-w-24 px-1 text-center text-sm font-medium tabular-nums" aria-live="polite">
          {formatMonth(date, i18n.language)}
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="size-10 rounded-l-none md:size-8"
          aria-label={t('budgets.next_month')}
          disabled={isCurrentMonth}
          onClick={() => onChange(addMonths(date, 1))}
        >
          <ChevronRight />
        </Button>
      </div>
      {!isCurrentMonth && (
        <Button variant="ghost" className="h-10 md:h-8" onClick={() => onChange(new Date())}>
          {t('budgets.this_month')}
        </Button>
      )}
    </div>
  );
}
