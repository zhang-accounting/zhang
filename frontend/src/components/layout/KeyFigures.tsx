import * as React from 'react';
import { Skeleton } from '@/components/ui/skeleton';
import { cn } from '@/lib/utils';

/** Grid of headline numbers (2 columns on mobile, `columns` on >= md). */
export function KeyFigures({ className, columns = 4, ...props }: React.ComponentProps<'dl'> & { columns?: 2 | 3 | 4 }) {
  return (
    <dl
      data-slot="key-figures"
      className={cn(
        'grid grid-cols-2 gap-3',
        columns === 2 && 'md:grid-cols-2',
        columns === 3 && 'md:grid-cols-3',
        columns === 4 && 'md:grid-cols-4',
        className,
      )}
      {...props}
    />
  );
}

export interface KeyFigureProps {
  label: React.ReactNode;
  value: React.ReactNode;
  /** Small muted line under the value (date, unit, comparison). */
  hint?: React.ReactNode;
  /** Extra content under the value, e.g. a progress bar. */
  children?: React.ReactNode;
  loading?: boolean;
  className?: string;
  valueClassName?: string;
}

/** One labelled number inside `KeyFigures`. Values are tabular and truncate instead of overflowing. */
export function KeyFigure({ label, value, hint, children, loading = false, className, valueClassName }: KeyFigureProps) {
  return (
    <div data-slot="key-figure" className={cn('flex min-w-0 flex-col gap-1 rounded-xl border bg-card p-3 md:p-4', className)}>
      <dt className="truncate text-xs font-medium text-muted-foreground">{label}</dt>
      <dd className={cn('truncate text-base font-semibold tabular-nums md:text-lg', valueClassName)}>
        {loading ? <Skeleton className="my-1 h-5 w-24" /> : value}
      </dd>
      {hint && !loading && <dd className="truncate text-xs text-muted-foreground">{hint}</dd>}
      {children && !loading && <dd>{children}</dd>}
    </div>
  );
}
