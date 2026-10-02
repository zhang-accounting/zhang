import * as React from 'react';
import type { LucideIcon } from 'lucide-react';
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty';
import { cn } from '@/lib/utils';

export interface EmptyStateProps {
  /** Lucide icon component (rendered in a muted tile) or any node. */
  icon?: LucideIcon | React.ReactNode;
  title: React.ReactNode;
  description?: React.ReactNode;
  /** Call to action, e.g. a `<Button>`. */
  action?: React.ReactNode;
  className?: string;
}

/** "Nothing here" placeholder for empty lists, empty search results and first-run states. */
export function EmptyState({ icon, title, description, action, className }: EmptyStateProps) {
  const media =
    icon == null || Array.isArray(icon) || React.isValidElement(icon) || (typeof icon !== 'object' && typeof icon !== 'function')
      ? icon
      : React.createElement(icon as LucideIcon);
  return (
    <Empty className={cn('border border-dashed', className)}>
      <EmptyHeader>
        {media != null && <EmptyMedia variant="icon">{media as React.ReactNode}</EmptyMedia>}
        <EmptyTitle>{title}</EmptyTitle>
        {description && <EmptyDescription>{description}</EmptyDescription>}
      </EmptyHeader>
      {action && <EmptyContent>{action}</EmptyContent>}
    </Empty>
  );
}
