import { Skeleton } from '../ui/skeleton';
import { TableRow, TableCell } from '../ui/table';

export function JournalListSkeleton() {
  return (
    <>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-[30%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-full mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-full mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-full mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-[30%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-full mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-[30%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-full mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-full mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={6}>
          <Skeleton className="h-5 w-[30%] mt-2.5 rounded-sm" />
        </TableCell>
      </TableRow>
    </>
  );
}
