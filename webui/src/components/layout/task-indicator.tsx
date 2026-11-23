import { Activity } from "lucide-react";
import { Link, useLocation } from "react-router-dom";
import { useTasksQuery } from "../../api/queries";
import { cn } from "../../lib/utils";

export function TaskIndicator() {
  const tasksQuery = useTasksQuery();
  const location = useLocation();
  const running = (tasksQuery.data || []).filter((task) => task.state === "running");

  if (location.pathname === "/tasks") return null;

  return (
    <Link
      to="/tasks"
      className={cn(
        "relative flex h-10 w-10 items-center justify-center rounded-full border transition-colors",
        running.length
          ? "border-primary/20 bg-primary-soft text-primary"
          : "border-border bg-surface text-fg-muted hover:text-fg",
      )}
      aria-label={running.length ? `${running.length} 个任务运行中` : "任务中心"}
    >
      <Activity size={18} className={running.length ? "animate-pulse" : undefined} />
      {running.length ? (
        <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[9px] font-semibold text-primary-fg">
          {running.length}
        </span>
      ) : null}
    </Link>
  );
}
