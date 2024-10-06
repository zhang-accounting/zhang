import { Skeleton } from '../ui/skeleton';
import { TableCell } from '../ui/table';
import { TableRow } from '../ui/table';

export function AccountListSkeleton() {
  return (
    <>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton className="h-5 w-[10%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton className="h-5 w-[20%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton className="h-5 w-[40%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton className="h-5 w-[10%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton className="h-5 w-[20%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={2}>
          <Skeleton className="h-5 w-[40%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
    </>
  );
}
