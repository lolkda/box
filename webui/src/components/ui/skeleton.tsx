import { cn } from "../../lib/utils";

export function Skeleton({ className }: { className?: string }) {
  return <div className={cn("animate-pulse rounded-lg bg-surface-muted", className)} aria-hidden />;
}

export function PageSkeleton() {
  return (
    <div className="space-y-4" aria-label="加载中">
      <div className="rounded-2xl border border-border bg-surface p-4">
        <Skeleton className="h-4 w-24" />
        <Skeleton className="mt-3 h-9 w-40" />
        <div className="mt-4 grid grid-cols-2 gap-2">
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
        </div>
      </div>
      <div className="rounded-2xl border border-border bg-surface p-4">
        <Skeleton className="h-4 w-20" />
        <Skeleton className="mt-3 h-12" />
        <Skeleton className="mt-2 h-12" />
      </div>
    </div>
  );
}
