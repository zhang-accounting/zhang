import { Skeleton } from '../ui/skeleton';

export function ErrorsSkeleton() {
  return (
    <>
      <Skeleton className="h-5 w-[10%] mt-2.5 rounded-sm" />
      <Skeleton className="h-5 w-[20%] mt-2.5 rounded-sm" />
      <Skeleton className="h-5 w-[40%] mt-2.5 rounded-sm" />
      <Skeleton className="h-5 w-[10%] mt-2.5 rounded-sm" />
    </>
  );
}
