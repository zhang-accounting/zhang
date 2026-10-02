import { Skeleton } from '../ui/skeleton';
import { TableCell, TableRow } from '../ui/table';

/** Table rows (desktop journals): a date header followed by entry rows, repeated. */
export function JournalListSkeleton({ groups = 3, rows = 3 }: { groups?: number; rows?: number }) {
  return (
    <>
      {Array.from({ length: groups }, (_, group) => [
        <TableRow key={`group-${group}`} className="bg-muted/40 hover:bg-muted/40">
          <TableCell colSpan={5} className="pl-4">
            <Skeleton className="h-3.5 w-36" />
          </TableCell>
        </TableRow>,
        ...Array.from({ length: rows }, (_, row) => (
          <TableRow key={`row-${group}-${row}`} className="hover:bg-transparent">
            <TableCell className="w-16 pl-4">
              <Skeleton className="h-3.5 w-10" />
            </TableCell>
            <TableCell className="w-24">
              <Skeleton className="h-5 w-14 rounded-full" />
            </TableCell>
            <TableCell>
              <Skeleton className="h-4 w-2/3 max-w-80" />
              <Skeleton className="mt-1.5 h-3 w-1/2 max-w-64" />
            </TableCell>
            <TableCell>
              <Skeleton className="ml-auto h-4 w-20" />
            </TableCell>
            <TableCell className="w-12" />
          </TableRow>
        )),
      ])}
    </>
  );
}

/** Card list (mobile journals). */
export function JournalCardsSkeleton({ groups = 2, rows = 4 }: { groups?: number; rows?: number }) {
  return (
    <div className="flex flex-col gap-4">
      {Array.from({ length: groups }, (_, group) => (
        <div key={group} className="flex flex-col gap-2">
          <Skeleton className="h-3.5 w-32" />
          <div className="divide-y overflow-hidden rounded-xl border bg-card">
            {Array.from({ length: rows }, (_, row) => (
              <div key={row} className="flex min-h-16 items-center gap-3 px-4 py-3">
                <Skeleton className="size-9 rounded-full" />
                <div className="flex flex-1 flex-col gap-1.5">
                  <Skeleton className="h-4 w-2/3" />
                  <Skeleton className="h-3 w-1/3" />
                </div>
                <Skeleton className="h-4 w-16" />
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
