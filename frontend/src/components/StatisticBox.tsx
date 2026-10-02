import type { LucideIcon } from 'lucide-react';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { useIsMobile } from '@/hooks/use-mobile';
import { cn } from '@/lib/utils';
import Amount from './Amount';
import { Card } from './ui/card';
import { Skeleton } from './ui/skeleton';

interface Props {
  /** i18n key of the label. */
  text: string;
  amount: string;
  currency?: string;
  detail?: unknown;
  /** Flip the sign (income and liabilities are stored as negative numbers). */
  negative?: boolean;
  /** Secondary line under the value. */
  hint?: React.ReactNode;
  icon?: LucideIcon;
  /** Short notation; defaults to `true` on mobile, where cards sit two per row. */
  compact?: boolean;
  loading?: boolean;
  className?: string;
}

/** KPI tile: label, one big tabular number and an optional hint. */
export default function StatisticBox({ text, amount, currency, negative, hint, icon: Icon, compact, loading, className }: Props) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const useCompact = compact ?? isMobile;

  return (
    <Card size="sm" className={cn('min-w-0 gap-1.5 px-3 md:gap-2 md:px-4 md:py-4', className)}>
      <div className="flex items-center justify-between gap-2 text-xs font-medium text-muted-foreground md:text-sm">
        <span className="truncate">{t(text)}</span>
        {Icon && <Icon className="size-4 shrink-0" aria-hidden />}
      </div>
      {loading ? (
        <Skeleton className="h-7 w-3/4 md:h-8" />
      ) : (
        <div className="truncate text-lg leading-tight font-semibold tracking-tight tabular-nums md:text-2xl">
          {currency ? <Amount amount={amount} negative={negative} currency={currency} compact={useCompact} /> : amount}
        </div>
      )}
      {hint && <div className="truncate text-xs text-muted-foreground">{loading ? <Skeleton className="h-3 w-1/2" /> : hint}</div>}
    </Card>
  );
}
