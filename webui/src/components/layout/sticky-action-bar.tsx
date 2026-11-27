import type { ReactNode } from "react";
import { cn } from "../../lib/utils";

export function StickyActionBar({
  label,
  status,
  children,
  className,
  actionsClassName,
}: {
  label: string;
  status?: ReactNode;
  children: ReactNode;
  className?: string;
  actionsClassName?: string;
}) {
  return (
    <div
      className={cn("pointer-events-none fixed inset-x-0 bottom-[calc(4.25rem+var(--safe-bottom))] z-30 px-4 pb-2", className)}
      role="region"
      aria-label={label}
    >
      <div className="pointer-events-auto mx-auto flex max-w-3xl items-center justify-between gap-3 rounded-2xl border border-border bg-surface/95 p-2 shadow-[0_-8px_24px_rgba(0,0,0,0.08)] backdrop-blur-xl">
        {status ? (
          <p className="sr-only sm:not-sr-only sm:block sm:min-w-0 sm:truncate sm:text-xs sm:text-fg-muted" aria-live="polite">
            {status}
          </p>
        ) : null}
        <div className={cn("flex w-full items-center justify-end gap-2 sm:w-auto", !status && "sm:ml-auto", actionsClassName)}>
          {children}
        </div>
      </div>
    </div>
  );
}
