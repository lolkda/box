import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Activity, Ban, ListChecks, RefreshCcw, ScrollText } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useTaskLogsQuery, useTasksQuery } from "../../api/queries";
import type { TaskInfo } from "../../api/types";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { EmptyState } from "../../components/ui/empty-state";
import { ErrorState } from "../../components/ui/error-state";
import { PageTitle } from "../../components/ui/page-title";
import { Sheet } from "../../components/ui/sheet";
import { PageSkeleton } from "../../components/ui/skeleton";
import { useToast } from "../../components/ui/toast";
import { formatDateTime, getErrorMessage, taskStateMeta } from "../../lib/utils";

function TaskCard({ task, onOpen, onCancel }: { task: TaskInfo; onOpen: () => void; onCancel: () => void }) {
  const meta = taskStateMeta(task.state);
  return (
    <Card className="overflow-hidden">
      <button type="button" onClick={onOpen} className="w-full p-4 text-left transition-colors hover:bg-surface-muted/60">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-[15px] font-semibold text-fg">{task.title || task.kind}</h2>
              <Badge tone={meta.tone}>{meta.label}</Badge>
            </div>
            <p className="mt-1 font-mono text-[10px] text-fg-subtle">{task.id}</p>
            <p className="mt-1 text-[11px] text-fg-muted">开始 {formatDateTime(task.started_at)}{task.finished_at ? ` · 结束 ${formatDateTime(task.finished_at)}` : ""}</p>
          </div>
          <ScrollText size={17} className="mt-0.5 shrink-0 text-fg-subtle" />
        </div>
      </button>
      {task.state === "running" && task.cancelable ? (
        <div className="border-t border-border px-4 py-2">
          <Button variant="ghost" size="sm" onClick={onCancel} icon={<Ban size={14} />}>取消任务</Button>
        </div>
      ) : null}
    </Card>
  );
}

function TaskDetailSheet({ task, onClose }: { task: TaskInfo | null; onClose: () => void }) {
  const logsQuery = useTaskLogsQuery(task?.id || null);
  const meta = task ? taskStateMeta(task.state) : null;

  return (
    <Sheet
      open={Boolean(task)}
      onOpenChange={(open) => { if (!open) onClose(); }}
      title={task?.title || "任务详情"}
      description={task ? `${task.kind} · ${task.id}` : undefined}
      footer={<Button variant="secondary" onClick={onClose}>关闭</Button>}
    >
      {task && meta ? (
        <div className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone={meta.tone}>{meta.label}</Badge>
            <Badge tone="neutral">退出码 {task.exit_code ?? "-"}</Badge>
            <Badge tone="neutral">{task.cancelable ? "可取消" : "不可取消"}</Badge>
          </div>
          <div className="max-h-[48dvh] overflow-auto rounded-xl bg-surface-muted p-3">
            {(logsQuery.data?.lines || []).length ? (
              <pre className="whitespace-pre-wrap break-all font-mono text-[10px] leading-5 text-fg-muted">
                {(logsQuery.data?.lines || []).join("\n")}
              </pre>
            ) : (
              <p className="text-xs text-fg-muted">{task.state === "running" ? "任务正在运行，暂无输出…" : "没有任务输出"}</p>
            )}
          </div>
        </div>
      ) : null}
    </Sheet>
  );
}

export function TasksPage() {
  const tasksQuery = useTasksQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [selected, setSelected] = useState<TaskInfo | null>(null);
  const [canceling, setCanceling] = useState<TaskInfo | null>(null);
  const [cancelLoading, setCancelLoading] = useState(false);

  useEffect(() => {
    if (!selected) return;
    const latest = (tasksQuery.data || []).find((task) => task.id === selected.id);
    if (latest) setSelected(latest);
  }, [tasksQuery.data, selected?.id]);

  const tasks = tasksQuery.data || [];
  const running = useMemo(() => tasks.filter((task) => task.state === "running").length, [tasks]);

  const cancel = async () => {
    if (!canceling) return;
    setCancelLoading(true);
    try {
      await api.taskCancel(canceling.id);
      toast({ title: "任务已取消", tone: "info" });
      setCanceling(null);
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
    } catch (error) {
      toast({ title: "取消任务失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setCancelLoading(false);
    }
  };

  return (
    <div className="space-y-4">
      <PageTitle
        title="任务中心"
        description="后台任务由 shell 托管，关闭页面后仍可继续运行"
        action={
          <>
            <Badge tone={running ? "info" : "neutral"}>{running ? `${running} 运行中` : "空闲"}</Badge>
            <Button variant="secondary" size="icon" aria-label="刷新任务" onClick={() => void tasksQuery.refetch()}>
              <RefreshCcw size={17} />
            </Button>
          </>
        }
      />
      {tasksQuery.isLoading ? (
        <PageSkeleton />
      ) : tasksQuery.error ? (
        <ErrorState error={tasksQuery.error} onRetry={() => void tasksQuery.refetch()} />
      ) : tasks.length === 0 ? (
        <EmptyState
          title="没有后台任务"
          description="更新订阅或内核时，任务会出现在这里，并且可以离开页面。"
          icon={<ListChecks size={22} />}
        />
      ) : (
        <div className="space-y-3">
          {tasks.map((task) => (
            <TaskCard key={task.id} task={task} onOpen={() => setSelected(task)} onCancel={() => setCanceling(task)} />
          ))}
        </div>
      )}
      <div className="flex items-start gap-2 rounded-2xl border border-border bg-surface px-4 py-3 text-[11px] leading-5 text-fg-muted">
        <Activity size={15} className="mt-0.5 shrink-0" />
        <span>同一时间只运行一个变更任务；内核替换阶段不可取消。页面刷新或重新打开后可继续查看进度。</span>
      </div>

      <TaskDetailSheet task={selected} onClose={() => setSelected(null)} />

      <ConfirmDialog
        open={Boolean(canceling)}
        onOpenChange={(open) => { if (!open) setCanceling(null); }}
        title="取消这个任务？"
        description={canceling?.title || ""}
        confirmLabel="取消任务"
        destructive
        loading={cancelLoading}
        onConfirm={() => void cancel()}
      />
    </div>
  );
}
