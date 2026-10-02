import * as React from 'react';
import { cn } from '@/lib/utils';

const WIDTH = {
  default: '',
  narrow: 'mx-auto max-w-3xl',
} as const;

export interface PageShellProps extends React.ComponentProps<'div'> {
  /** `default` fills the shell column (max-w-7xl); `narrow` (max-w-3xl) suits forms and settings. */
  width?: keyof typeof WIDTH;
}

/** Root of every page: vertical stack with consistent section spacing. The AppShell already provides paddings. */
export function PageShell({ width = 'default', className, ...props }: PageShellProps) {
  return <div data-slot="page-shell" className={cn('flex w-full min-w-0 flex-col gap-4 md:gap-6', WIDTH[width], className)} {...props} />;
}
