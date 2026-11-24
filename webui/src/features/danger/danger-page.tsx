import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { AlertTriangle, FileX2, RotateCcw, ScrollText, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useTaskStart } from "../../api/queries";
import { Button } from "../../components/ui/button";
import { Card, CardBody, CardHeader } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { PageTitle } from "../../components/ui/page-title";
import { useToast } from "../../components/ui/toast";
import { getErrorMessage } from "../../lib/utils";

type DangerOp = "delete-config" | "clear-logs" | "reset-subs" | "upkernel-mihomo";

interface DangerItem {
  id: DangerOp;
  title: string;
  description: string;
  confirmDescription: string;
  icon: typeof Trash2;
}

const items: DangerItem[] = [
  {
    id: "delete-config",
    title: "删除当前配置",
    description: "把 config.yaml 移走，核心将无法启动",
    confirmDescription: "会重命名当前 config.yaml 并保留一份带时间戳的副本；后续启动可能失败。此操作不可撤销。",
    icon: FileX2,
  },
  {
    id: "clear-logs",
    title: "清空日志",
    description: "清空 runs.log、tool.log 和核心日志",
    confirmDescription: "会清空运行日志、工具日志和当前核心日志。此操作不可撤销。",
    icon: ScrollText,
  },
  {
    id: "reset-subs",
    title: "重置订阅",
    description: "清空订阅清单和所有 provider 文件",
    confirmDescription: "会清空 subscriptions.list 和所有 provider YAML。此操作不可撤销，且不会自动备份。",
    icon: RotateCcw,
  },
  {
    id: "upkernel-mihomo",
    title: "更新 mihomo 核心",
    description: "从模块内置 GitHub 源下载并替换核心",
    confirmDescription: "会下载并替换 mihomo 核心，期间服务可能重启。任务启动后不可取消。",
    icon: AlertTriangle,
  },
];

export function DangerPage() {
  const queryClient = useQueryClient();
  const startTask = useTaskStart();
  const navigate = useNavigate();
  const toast = useToast();
  const [pending, setPending] = useState<DangerItem | null>(null);
  const [loading, setLoading] = useState(false);

  const run = async () => {
    if (!pending) return;
    setLoading(true);
    try {
      if (pending.id === "delete-config") {
        await api.deleteConfig();
        toast({ title: "配置已移走", tone: "success" });
        await queryClient.invalidateQueries({ queryKey: queryKeys.config });
        await queryClient.invalidateQueries({ queryKey: queryKeys.status });
      } else if (pending.id === "clear-logs") {
        await api.clearLogs();
        toast({ title: "日志已清空", tone: "success" });
        await queryClient.invalidateQueries({ queryKey: queryKeys.logs });
      } else if (pending.id === "reset-subs") {
        await api.resetSubscriptions();
        toast({ title: "订阅已重置", tone: "success" });
        await queryClient.invalidateQueries({ queryKey: queryKeys.subscriptions });
        await queryClient.invalidateQueries({ queryKey: queryKeys.config });
        await queryClient.invalidateQueries({ queryKey: queryKeys.status });
      } else {
        await startTask("upkernel", "mihomo");
        toast({ title: "已开始更新 mihomo 核心", tone: "info" });
        setPending(null);
        navigate("/tasks");
        return;
      }
      setPending(null);
    } catch (error) {
      toast({ title: "操作失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-4">
      <PageTitle title="危险操作" description="高风险操作，全部需要二次确认，不会自动备份" />
      <div className="flex items-start gap-2 rounded-2xl border border-danger/20 bg-danger-soft px-4 py-3 text-xs leading-5 text-danger">
        <AlertTriangle size={17} className="mt-0.5 shrink-0" />
        <p>以下操作不可撤销，请确认当前配置和订阅已自行备份。</p>
      </div>
      <Card>
        <CardHeader title="维护操作" description="点击执行后会再次确认影响" />
        <CardBody className="p-0">
          <div className="divide-y divide-border">
            {items.map((item) => {
              const Icon = item.icon;
              return (
                <div key={item.id} className="flex items-center gap-3 px-4 py-3.5">
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-danger-soft text-danger">
                    <Icon size={17} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium text-fg">{item.title}</p>
                    <p className="mt-0.5 text-[11px] text-fg-muted">{item.description}</p>
                  </div>
                  <Button variant="danger-outline" size="sm" onClick={() => setPending(item)}>执行</Button>
                </div>
              );
            })}
          </div>
        </CardBody>
      </Card>
      <ConfirmDialog
        open={Boolean(pending)}
        onOpenChange={(open) => { if (!open) setPending(null); }}
        title={pending?.title || "确认操作"}
        description={pending?.confirmDescription}
        confirmLabel="确认执行"
        destructive
        loading={loading}
        onConfirm={() => void run()}
      />
    </div>
  );
}
