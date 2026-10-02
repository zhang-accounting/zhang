import { Skeleton } from '../ui/skeleton';

export function ErrorsSkeleton() {
  return (
    <div className="flex flex-col gap-3">
      {[60, 85, 45, 70].map((width, index) => (
        <div key={index} className="flex flex-col gap-1.5">
          <Skeleton className="h-4" style={{ width: `${width}%` }} />
          <Skeleton className="h-3 w-1/3" />
        </div>
      ))}
    </div>
  );
}
