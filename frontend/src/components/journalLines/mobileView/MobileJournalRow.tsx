import * as React from 'react';
import { JournalItem } from '@/api/types';
import { cn } from '@/lib/utils';
import { JournalStatusBadge, JournalTypeIcon } from '../JournalBits';
import { journalStatus } from '../journal-utils';

interface Props {
  data: JournalItem;
  title: React.ReactNode;
  subtitle: React.ReactNode;
  /** Right column (amounts). */
  trailing: React.ReactNode;
  /** Extra line under the subtitle (tags, links). */
  extra?: React.ReactNode;
  onOpen: () => void;
}

/** Tappable mobile journal row (>= 64px tall): type icon, title + meta, amount on the right. */
export function MobileJournalRow({ data, title, subtitle, trailing, extra, onOpen }: Props) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(event) => {
        // Only the row itself: Enter / Space on an inner control (tag chip) must keep its own behaviour.
        if (event.target !== event.currentTarget) return;
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onOpen();
        }
      }}
      className={cn(
        'flex min-h-16 w-full cursor-pointer items-center gap-3 px-4 py-3 text-left outline-none transition-colors active:bg-muted',
        'focus-visible:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
      )}
    >
      <JournalTypeIcon type={data.type} status={journalStatus(data)} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="truncate text-sm font-medium">{title}</div>
        <div className="truncate text-xs text-muted-foreground">{subtitle}</div>
        {extra}
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1 text-sm">
        {trailing}
        <JournalStatusBadge data={data} />
      </div>
    </div>
  );
}
