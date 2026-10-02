import * as React from 'react';
import { cn } from '@/lib/utils';

/** Titled block inside the preview drawer / dialog. */
export function PreviewSection({
  title,
  action,
  children,
  className,
}: {
  title: React.ReactNode;
  action?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={cn('flex flex-col gap-2', className)}>
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  );
}

/** Bordered list container for postings / key-value rows. */
export function PreviewList({ children }: { children: React.ReactNode }) {
  return <div className="divide-y rounded-lg border bg-card">{children}</div>;
}

/** Key / value row inside a `PreviewList`. */
export function PreviewRow({ label, children }: { label: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex min-h-10 items-center justify-between gap-4 px-3 py-2 text-sm">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 text-right break-words">{children}</span>
    </div>
  );
}

/** Account + amount row with the balance after the posting underneath. */
export function PostingRow({ account, amount, balance }: { account: string; amount: React.ReactNode; balance?: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-3 px-3 py-2.5 text-sm">
      <span className="min-w-0 break-all">{account}</span>
      <span className="flex shrink-0 flex-col items-end gap-0.5">
        <span className="font-medium">{amount}</span>
        {balance && <span className="text-xs text-muted-foreground">{balance}</span>}
      </span>
    </div>
  );
}

/** Summary block at the top of a preview: badges, the headline amount, title and date. */
export function PreviewHeader({
  badges,
  amount,
  title,
  meta,
}: {
  badges?: React.ReactNode;
  amount?: React.ReactNode;
  title?: React.ReactNode;
  meta?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5 rounded-lg bg-muted/40 p-4">
      {badges && <div className="flex flex-wrap items-center gap-1.5">{badges}</div>}
      {amount && <div className="text-2xl font-semibold tracking-tight">{amount}</div>}
      {title && <div className="text-sm break-words">{title}</div>}
      {meta && <div className="text-xs text-muted-foreground tabular-nums">{meta}</div>}
    </div>
  );
}
