import { useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Copy, Pause, Play, RefreshCcw, Search, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useLogsQuery } from "../../api/queries";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card, CardBody } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { EmptyState } from "../../components/ui/empty-state";
import { ErrorState } from "../../components/ui/error-state";
import { Input } from "../../components/ui/input";
import { PageTitle } from "../../components/ui/page-title";
import { useToast } from "../../components/ui/toast";
import { cn, getErrorMessage } from "../../lib/utils";

function levelOf(line: string): "Info" | "Warn" | "Error" | "Other" {
  if (line.includes("[Error]")) return "Error";
  if (line.includes("[Warn]") || line.includes("[Warning]")) return "Warn";
  if (line.includes("[Info]")) return "Info";
  return "Other";
}

function messageOf(line: string): string {
  return line.replace(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?:Info|Warn|Warning|Error|Debug)\]:\s?/, "");
}

function timeOf(line: string): string {
  return line.slice(0, 19);
}

async function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  document.execCommand("copy");
  area.remove();
}

export function LogsPage() {
  const [paused, setPaused] = useState(false);
  const [level, setLevel] = useState("all");
  const [query, setQuery] = useState("");
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const logsQuery = useLogsQuery({ paused });
  const queryClient = useQueryClient();
  const toast = useToast();
  const scrollRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    const term = query.trim().toLowerCase();
    return (logsQuery.data?.lines || []).filter((line) => {
      if (level !== "all" && levelOf(line) !== level) return false;
      if (term && !line.toLowerCase().includes(term)) return false;
      return true;
    });
  }, [logsQuery.data?.lines, level, query]);

  useEffect(() => {
    if (paused) return;
    const element = scrollRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [filtered.length, paused]);

  const clear = async () => {
    setClearing(true);
    try {
      await api.clearLogs();
      toast({ title: "日志已清空", tone: "success" });
      setClearOpen(false);
      await queryClient.invalidateQueries({ queryKey: queryKeys.logs });
    } catch (error) {
      toast({ title: "清空日志失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setClearing(false);
    }
  };

  const copy = async () => {
    try {
      await copyText(filtered.join("\n"));
      toast({ title: `已复制 ${filtered.length} 行日志`, tone: "success" });
    } catch (error) {
      toast({ title: "复制失败", description: getErrorMessage(error), tone: "error" });
    }
  };

  return (
    <div className="space-y-4">
      <PageTitle
        title="日志"
        description={paused ? "自动刷新已暂停" : "每 2 秒自动刷新"}
        action={
          <>
            <Button variant="secondary" size="icon" aria-label={paused ? "恢复自动刷新" : "暂停自动刷新"} onClick={() => setPaused((value) => !value)}>
              {paused ? <Play size={17} /> : <Pause size={17} />}
            </Button>
            <Button variant="secondary" size="icon" aria-label="手动刷新" onClick={() => void logsQuery.refetch()}>
              <RefreshCcw size={17} />
            </Button>
          </>
        }
      />

      <div className="flex gap-2">
        <div className="relative flex-1">
          <Search size={15} className="absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle" />
          <Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索日志" className="pl-9" />
        </div>
        <select
          value={level}
          onChange={(event) => setLevel(event.currentTarget.value)}
          className="h-11 rounded-xl border border-border bg-surface-muted px-3 text-xs text-fg focus:border-primary focus:outline-none"
          aria-label="日志级别"
        >
          <option value="all">全部级别</option>
          <option value="Info">Info</option>
          <option value="Warn">Warn</option>
          <option value="Error">Error</option>
        </select>
      </div>

      {logsQuery.isLoading ? (
        <Card><CardBody><p className="text-xs text-fg-muted">正在读取日志…</p></CardBody></Card>
      ) : logsQuery.error && !logsQuery.data ? (
        <ErrorState error={logsQuery.error} onRetry={() => void logsQuery.refetch()} />
      ) : (
        <Card className="overflow-hidden">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-3">
            <div className="flex items-center gap-2">
              <Badge tone="neutral">{filtered.length} 行</Badge>
              <span className="truncate font-mono text-[10px] text-fg-subtle">{logsQuery.data?.file}</span>
            </div>
            <div className="flex gap-1">
              <Button variant="ghost" size="sm" onClick={() => void copy()} icon={<Copy size={14} />}>复制</Button>
              <Button variant="ghost" size="sm" onClick={() => setClearOpen(true)} icon={<Trash2 size={14} />}>清空</Button>
            </div>
          </div>
          <div ref={scrollRef} className="max-h-[60dvh] overflow-auto bg-surface-muted/50 p-3">
            {filtered.length ? (
              <div className="space-y-1">
                {filtered.map((line, index) => {
                  const lineLevel = levelOf(line);
                  return (
                    <div key={`${line}-${index}`} className="rounded-lg px-2 py-1.5 hover:bg-surface">
                      <div className="flex gap-2">
                        <span className={cn("mt-1 h-1.5 w-1.5 shrink-0 rounded-full", lineLevel === "Error" ? "bg-danger" : lineLevel === "Warn" ? "bg-warning" : "bg-info")} />
                        <div className="min-w-0">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="font-mono text-[10px] text-fg-subtle">{timeOf(line)}</span>
                            <Badge tone={lineLevel === "Error" ? "danger" : lineLevel === "Warn" ? "warning" : "info"}>{lineLevel}</Badge>
                          </div>
                          <p className="mt-0.5 break-all font-mono text-[11px] leading-5 text-fg-muted">{messageOf(line)}</p>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              <EmptyState title="没有匹配的日志" description="调整级别或关键词过滤。" />
            )}
          </div>
        </Card>
      )}

      <ConfirmDialog
        open={clearOpen}
        onOpenChange={setClearOpen}
        title="清空日志？"
        description="会清空 runs.log、tool.log 和当前核心日志。此操作不可撤销。"
        confirmLabel="清空"
        destructive
        loading={clearing}
        onConfirm={() => void clear()}
      />
    </div>
  );
}
