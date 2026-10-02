import { ReactNode } from 'react';
import { cn } from '@/lib/utils';

interface Props {
  children: ReactNode;
  className?: string;
}

/** Key / value row: first child is the muted label, last child the right-aligned value. Consecutive rows get a dashed divider. */
export default function DashLine({ children, className }: Props) {
  return (
    <div
      className={cn(
        'flex min-h-10 items-center justify-between gap-4 py-2 text-sm [&+&]:border-t [&+&]:border-dashed',
        '[&>*:first-child]:shrink-0 [&>*:first-child]:text-muted-foreground [&>*:last-child]:min-w-0 [&>*:last-child]:text-right',
        className,
      )}
    >
      {children}
    </div>
  );
}
