import { Skeleton } from '../ui/skeleton';

/** Rows shaped like `JournalRow` (title, chips, amount) without the day heading. */
export function JournalRowsSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <>
      {Array.from({ length: rows }, (_, row) => (
        <div key={row} className="flex items-center gap-3 px-3.5 py-2.5">
          <div className="flex flex-1 flex-col gap-1.5">
            <Skeleton className="h-4 w-2/3 max-w-72" />
            <div className="flex gap-1.5">
              <Skeleton className="h-4 w-16" />
              <Skeleton className="h-4 w-24" />
            </div>
          </div>
          <Skeleton className="h-4 w-16" />
        </div>
      ))}
    </>
  );
}

/** Journals page while loading: day headings followed by a card of rows, repeated. */
export function JournalDaysSkeleton({ groups = 3, rows = 3 }: { groups?: number; rows?: number }) {
  return (
    <div className="flex flex-col gap-5">
      {Array.from({ length: groups }, (_, group) => (
        <div key={group} className="flex flex-col gap-2">
          <Skeleton className="h-4 w-28" />
          <div className="divide-y overflow-hidden rounded-lg border bg-card">
            <JournalRowsSkeleton rows={rows} />
          </div>
        </div>
      ))}
    </div>
  );
}
