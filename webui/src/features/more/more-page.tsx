import { ChevronRight, Database, FileWarning, ListChecks, ScrollText, Settings2 } from "lucide-react";
import { Link } from "react-router-dom";
import { useTasksQuery } from "../../api/queries";
import { Badge } from "../../components/ui/badge";
import { Card, CardBody } from "../../components/ui/card";
import { PageTitle } from "../../components/ui/page-title";

const items = [
  { to: "/settings", title: "设置", description: "网络、代理、性能与更新选项", icon: Settings2 },
  { to: "/providers", title: "规则集", description: "规则来源、格式与更新间隔", icon: Database },
  { to: "/logs", title: "日志", description: "运行日志、自动刷新与过滤", icon: ScrollText },
  { to: "/tasks", title: "任务中心", description: "订阅更新、内核更新等后台任务", icon: ListChecks },
  { to: "/danger", title: "危险操作", description: "删除配置、清空日志、更新核心", icon: FileWarning, danger: true },
];

export function MorePage() {
  const tasksQuery = useTasksQuery();
  const running = (tasksQuery.data || []).filter((task) => task.state === "running").length;

  return (
    <div className="space-y-4">
      <PageTitle title="更多" description="设置、规则集、日志、任务与危险操作" />
      <Card>
        <CardBody className="p-0">
          <div className="divide-y divide-border">
            {items.map((item) => {
              const Icon = item.icon;
              return (
                <Link key={item.to} to={item.to} className="flex items-center gap-3 px-4 py-3.5 transition-colors hover:bg-surface-muted">
                  <span className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ${item.danger ? "bg-danger-soft text-danger" : "bg-surface-muted text-fg-muted"}`}>
                    <Icon size={19} />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className={`block text-sm font-medium ${item.danger ? "text-danger" : "text-fg"}`}>{item.title}</span>
                    <span className="mt-0.5 block text-[11px] text-fg-muted">{item.description}</span>
                  </span>
                  {item.to === "/tasks" && running ? <Badge tone="info">{running} 运行中</Badge> : null}
                  <ChevronRight size={17} className="shrink-0 text-fg-subtle" />
                </Link>
              );
            })}
          </div>
        </CardBody>
      </Card>
      <p className="px-1 text-center text-[10px] text-fg-subtle">Box for Root · KernelSU WebUI</p>
    </div>
  );
}
