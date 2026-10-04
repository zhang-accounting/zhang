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
  /** Flip the sign (income and liabilities are stored as negative numbers). */
  negative?: boolean;
  /** Secondary line under the value. */
  hint?: React.ReactNode;
  icon?: LucideIcon;
  /** A small action in the header, such as opening the query behind the figure. */
  action?: React.ReactNode;
  /** Colour the value as money coming in (`positive`) or going out (`negative`). */
  tone?: 'positive' | 'negative';
  /** Short notation; defaults to `true` on mobile, where cards sit two per row. */
  compact?: boolean;
  loading?: boolean;
  className?: string;
}

/** KPI tile: label, one big tabular number and an optional hint. */
export default function StatisticBox({ text, amount, currency, negative, hint, icon: Icon, action, tone, compact, loading, className }: Props) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const useCompact = compact ?? isMobile;

  return (
    <Card size="sm" className={cn('min-w-0 gap-1 px-3 md:px-3.5 md:py-3', className)}>
      <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground md:text-[13px]">
        <span className="truncate">{t(text)}</span>
        <span className="flex shrink-0 items-center gap-1">
          {action}
          {Icon && <Icon className="size-4 shrink-0" aria-hidden />}
        </span>
      </div>
      {loading ? (
        <Skeleton className="h-7 w-3/4 md:h-8" />
      ) : (
        <div
          className={cn(
            'truncate text-lg leading-tight font-normal tabular-nums md:text-xl',
            tone === 'positive' && 'text-positive',
            tone === 'negative' && 'text-negative',
          )}
        >
          {currency ? <Amount amount={amount} negative={negative} currency={currency} compact={useCompact} /> : amount}
        </div>
      )}
      {hint && <div className="truncate text-xs text-muted-foreground">{loading ? <Skeleton className="h-3 w-1/2" /> : hint}</div>}
    </Card>
  );
}
