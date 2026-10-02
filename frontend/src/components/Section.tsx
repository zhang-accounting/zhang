import * as React from 'react';
import { cn } from '@/lib/utils';
import { Card } from './ui/card';

interface Props {
  title: React.ReactNode;
  description?: React.ReactNode;
  /** Actions / meta aligned to the right of the title. */
  rightSection?: React.ReactNode;
  children: React.ReactNode;
  /** Let the content run edge to edge (lists, tables); the header gets a bottom border instead. */
  noPadding?: boolean;
  /** Border under the header; defaults to `noPadding`. */
  divider?: boolean;
  className?: string;
  contentClassName?: string;
}

/** Titled card used to group page content (charts, lists, key/value blocks). */
export default function Section({ children, title, description, rightSection, noPadding, divider = noPadding, className, contentClassName }: Props) {
  return (
    <Card className={cn('gap-0 py-0', className)}>
      <div className={cn('flex min-h-12 items-center justify-between gap-3 px-4 py-3', divider && 'border-b')}>
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="truncate text-sm font-medium">{title}</h2>
          {description && <p className="text-xs text-muted-foreground">{description}</p>}
        </div>
        {rightSection && <div className="flex shrink-0 items-center gap-2">{rightSection}</div>}
      </div>
      <div className={cn('min-w-0', !noPadding && 'px-4 pb-4', contentClassName)}>{children}</div>
    </Card>
  );
}
