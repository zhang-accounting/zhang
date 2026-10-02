import * as React from 'react';
import { cn } from '@/lib/utils';

export interface PageHeaderProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  /** Buttons / menus aligned right on >= sm, wrapped under the title on mobile. */
  actions?: React.ReactNode;
  /** Optional second row (filters, tabs, summary chips). */
  children?: React.ReactNode;
  className?: string;
}

/** Page title block. Renders an `<h1>`; keep one per page. */
export function PageHeader({ title, description, actions, children, className }: PageHeaderProps) {
  return (
    <header data-slot="page-header" className={cn('flex flex-col gap-3', className)}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="flex min-w-0 flex-col gap-1">
          <h1 className="truncate text-xl font-semibold tracking-tight md:text-2xl">{title}</h1>
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {children}
    </header>
  );
}
