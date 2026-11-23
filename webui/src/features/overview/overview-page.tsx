import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import {
  Activity,
  ArrowUpRight,
  Gauge,
  Layers3,
  ListTree,
  Play,
  RefreshCcw,
  RotateCw,
  Square,
  RadioTower,
} from "lucide-react";
import { api } from "../../api/client";
import type { ServiceAction } from "../../api/types";
import { queryKeys, useStatusQuery } from "../../api/queries";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card } from "../../components/ui/card";
import { ErrorState } from "../../components/ui/error-state";
import { PageSkeleton } from "../../components/ui/skeleton";
import { PageTitle } from "../../components/ui/page-title";
import { useToast } from "../../components/ui/toast";
import { cn, getErrorMessage } from "../../lib/utils";

const actionLabels: Record<ServiceAction, string> = {
  start: "启动",
  stop: "停止",
  restart: "重启",
  reload: "重载",
};

export function OverviewPage() {
  const statusQuery = useStatusQuery({ live: true });
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const [pendingAction, setPendingAction] = useState<ServiceAction | null>(null);

  const runAction = async (action: ServiceAction) => {
    setPendingAction(action);
    try {
      await api.service(action);
      toast({ title: `服务${actionLabels[action]}成功`, tone: "success" });
      await queryClient.invalidateQueries({ queryKey: queryKeys.status });
    } catch (error) {
      toast({
        title: `服务${actionLabels[action]}失败`,
        description: getErrorMessage(error),
        tone: "error",
      });
    } finally {
      setPendingAction(null);
    }
  };

  const status = statusQuery.data;

  return (
    <div className="space-y-4">
      <PageTitle
        title="概览"
        description="运行状态与快捷操作"
        action={
          <Button
            variant="secondary"
            size="icon"
            aria-label="刷新概览"
            onClick={() => {
              void queryClient.invalidateQueries({ queryKey: queryKeys.status });
            }}
          >
            <RefreshCcw size={17} />
          </Button>
        }
      />

      {statusQuery.isLoading ? (
        <PageSkeleton />
      ) : statusQuery.error ? (
        <ErrorState error={statusQuery.error} onRetry={() => void statusQuery.refetch()} />
      ) : status ? (
        <>
          <Card className="overflow-hidden">
            <div className="relative overflow-hidden bg-gradient-to-br from-primary/10 via-surface to-surface px-4 pb-4 pt-5">
              <div className="absolute -right-10 -top-12 h-36 w-36 rounded-full bg-primary/10 blur-2xl" aria-hidden />
              <div className="relative flex items-start justify-between gap-3">
                <div>
                  <p className="text-[11px] font-medium uppercase tracking-[0.16em] text-fg-muted">Box for Root</p>
                  <div className="mt-2 flex items-center gap-2">
                    <span
                      className={cn(
                        "h-2.5 w-2.5 rounded-full",
                        status.running ? "bg-success shadow-[0_0_0_4px_var(--color-success-soft)]" : "bg-fg-subtle",
                      )}
                    />
                    <h2 className="text-2xl font-semibold tracking-tight">{status.running ? "运行中" : "已停止"}</h2>
                  </div>
                  <p className="mt-1 text-xs text-fg-muted">
                    {status.core} · {status.mode} · PID {status.pid ?? "-"}
                  </p>
                </div>
                <Badge tone={status.running ? "success" : "neutral"} className="mt-1">
                  {status.running ? "服务正常" : "服务停止"}
                </Badge>
              </div>

              <div className="relative mt-5 grid grid-cols-2 gap-2">
                <div className="rounded-xl border border-border/70 bg-surface/80 px-3 py-2.5">
                  <p className="text-[10px] text-fg-muted">配置</p>
                  <p className="mt-0.5 truncate text-xs font-medium">{status.config_exists ? status.config : "配置文件缺失"}</p>
                </div>
                <div className="rounded-xl border border-border/70 bg-surface/80 px-3 py-2.5">
                  <p className="text-[10px] text-fg-muted">订阅</p>
                  <p className="mt-0.5 text-xs font-medium">{status.subscriptions} 个</p>
                </div>
                <div className="rounded-xl border border-border/70 bg-surface/80 px-3 py-2.5">
                  <p className="text-[10px] text-fg-muted">活动任务</p>
                  <p className="mt-0.5 text-xs font-medium">{status.active_tasks} 个</p>
                </div>
                <div className="rounded-xl border border-border/70 bg-surface/80 px-3 py-2.5">
                  <p className="text-[10px] text-fg-muted">模式</p>
                  <p className="mt-0.5 text-xs font-medium">{status.mode}</p>
                </div>
              </div>
            </div>
            <div className="grid grid-cols-4 gap-2 border-t border-border bg-surface p-3">
              <Button
                size="sm"
                variant="primary"
                loading={pendingAction === "start"}
                disabled={status.running || Boolean(pendingAction)}
                onClick={() => void runAction("start")}
                icon={<Play size={14} />}
              >
                启动
              </Button>
              <Button
                size="sm"
                variant="secondary"
                loading={pendingAction === "stop"}
                disabled={!status.running || Boolean(pendingAction)}
                onClick={() => void runAction("stop")}
                icon={<Square size={13} />}
              >
                停止
              </Button>
              <Button
                size="sm"
                variant="secondary"
                loading={pendingAction === "restart"}
                disabled={!status.running || Boolean(pendingAction)}
                onClick={() => void runAction("restart")}
                icon={<RotateCw size={14} />}
              >
                重启
              </Button>
              <Button
                size="sm"
                variant="secondary"
                loading={pendingAction === "reload"}
                disabled={!status.running || Boolean(pendingAction)}
                onClick={() => void runAction("reload")}
                icon={<RefreshCcw size={14} />}
              >
                重载
              </Button>
            </div>
          </Card>

          <div className="grid grid-cols-2 gap-3">
            <button
              type="button"
              onClick={() => navigate("/subscriptions")}
              className="group rounded-2xl border border-border bg-surface p-4 text-left shadow-sm transition-colors hover:border-primary/30 hover:bg-primary-soft/20"
            >
              <div className="flex items-center justify-between">
                <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary-soft text-primary"><RadioTower size={18} /></span>
                <ArrowUpRight size={16} className="text-fg-subtle transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" />
              </div>
              <p className="mt-3 text-sm font-semibold">订阅</p>
              <p className="mt-0.5 text-xs text-fg-muted">{status.subscriptions} 个订阅源</p>
            </button>
            <button
              type="button"
              onClick={() => navigate("/groups")}
              className="group rounded-2xl border border-border bg-surface p-4 text-left shadow-sm transition-colors hover:border-primary/30 hover:bg-primary-soft/20"
            >
              <div className="flex items-center justify-between">
                <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-info-soft text-info"><Layers3 size={18} /></span>
                <ArrowUpRight size={16} className="text-fg-subtle transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" />
              </div>
              <p className="mt-3 text-sm font-semibold">策略组</p>
              <p className="mt-0.5 text-xs text-fg-muted">管理节点分组</p>
            </button>
            <button
              type="button"
              onClick={() => navigate("/rules")}
              className="group rounded-2xl border border-border bg-surface p-4 text-left shadow-sm transition-colors hover:border-primary/30 hover:bg-primary-soft/20"
            >
              <div className="flex items-center justify-between">
                <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-warning-soft text-warning"><ListTree size={18} /></span>
                <ArrowUpRight size={16} className="text-fg-subtle transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" />
              </div>
              <p className="mt-3 text-sm font-semibold">规则</p>
              <p className="mt-0.5 text-xs text-fg-muted">按顺序匹配流量</p>
            </button>
            <button
              type="button"
              onClick={() => navigate("/logs")}
              className="group rounded-2xl border border-border bg-surface p-4 text-left shadow-sm transition-colors hover:border-primary/30 hover:bg-primary-soft/20"
            >
              <div className="flex items-center justify-between">
                <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-success-soft text-success"><Activity size={18} /></span>
                <ArrowUpRight size={16} className="text-fg-subtle transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" />
              </div>
              <p className="mt-3 text-sm font-semibold">日志</p>
              <p className="mt-0.5 text-xs text-fg-muted">最近运行记录</p>
            </button>
          </div>

          <div className="flex items-center justify-between rounded-2xl border border-border bg-surface px-4 py-3 text-xs text-fg-muted">
            <span className="flex items-center gap-2"><Gauge size={16} /> 核心 {status.core}</span>
            <a
              href="http://127.0.0.1:9090/ui/"
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 font-medium text-primary"
            >
              打开面板 <ArrowUpRight size={14} />
            </a>
          </div>
        </>
      ) : null}
    </div>
  );
}
