import { Skeleton } from '../ui/skeleton';

/** Account type cards with indented rows. */
export function AccountListSkeleton({ groups = 3 }: { groups?: number }) {
  return (
    <div className="flex flex-col gap-4">
      {Array.from({ length: groups }, (_, group) => (
        <div key={group} className="overflow-hidden rounded-xl border bg-card">
          <div className="flex items-center justify-between border-b px-4 py-3">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-24" />
          </div>
          {[40, 55, 35].map((width, row) => (
            <div key={row} className="flex min-h-12 items-center justify-between gap-4 px-4 py-2">
              <Skeleton className="h-4" style={{ width: `${width}%` }} />
              <Skeleton className="h-4 w-20" />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
