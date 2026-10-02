import * as React from 'react';
import { cn } from '@/lib/utils';

interface SettingsSectionProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  /** Right-aligned header action (e.g. a button). */
  action?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  /** Render children without the bordered row container (for grids / empty states). */
  bare?: boolean;
}

/** A titled group of setting rows rendered as one bordered card with dividers. */
export function SettingsSection({ title, description, action, children, className, bare = false }: SettingsSectionProps) {
  return (
    <section className={cn('flex flex-col gap-3', className)}>
      <div className="flex items-end justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-sm font-semibold">{title}</h2>
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </div>
        {action && <div className="shrink-0">{action}</div>}
      </div>
      {bare ? children : <div className="divide-y overflow-hidden rounded-xl border bg-card">{children}</div>}
    </section>
  );
}

interface SettingRowProps {
  label: React.ReactNode;
  description?: React.ReactNode;
  /** Control or value, right-aligned on >= sm and stacked under the label on mobile. */
  children?: React.ReactNode;
  /** id of the control, to associate the label. */
  htmlFor?: string;
  /** Keep label and value on one line on mobile too (short read-only values). */
  inline?: boolean;
  className?: string;
}

/** Label + description on the left, control on the right (stacked on mobile). */
export function SettingRow({ label, description, children, htmlFor, inline = false, className }: SettingRowProps) {
  const Label = htmlFor ? 'label' : 'div';
  return (
    <div
      className={cn(
        'flex gap-3 p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-6',
        inline ? 'flex-row items-center justify-between' : 'flex-col',
        className,
      )}
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        <Label htmlFor={htmlFor} className="text-sm font-medium">
          {label}
        </Label>
        {description && <p className="text-sm text-muted-foreground">{description}</p>}
      </div>
      {children !== undefined && (
        <div className={cn('flex min-w-0 items-center gap-2 sm:shrink-0 sm:justify-end', inline && 'shrink-0 justify-end')}>{children}</div>
      )}
    </div>
  );
}
