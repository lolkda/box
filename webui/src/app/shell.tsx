import { useCallback, useEffect } from "react";
import { Outlet, useLocation, useNavigate, useRouteError } from "react-router-dom";
import { ChevronLeft, ShieldAlert } from "lucide-react";
import { isKernelsuAvailable, supportsAsyncExec } from "../api/client";
import { useStatusQuery, useTaskCompletionWatcher } from "../api/queries";
import { useToast } from "../components/ui/toast";
import { BottomNav } from "../components/layout/bottom-nav";
import { PageSkeleton } from "../components/ui/skeleton";
import { getErrorMessage } from "../lib/utils";
import { Button } from "../components/ui/button";

const pageMeta: Record<string, { title: string; back?: string }> = {
  "/": { title: "概览" },
  "/subscriptions": { title: "订阅" },
  "/subscriptions/rule-profile": { title: "订阅" },
  "/groups": { title: "策略" },
  "/rules": { title: "规则" },
  "/more": { title: "更多" },
  "/settings": { title: "设置", back: "/more" },
  "/providers": { title: "规则集", back: "/more" },
  "/logs": { title: "日志", back: "/more" },
  "/tasks": { title: "任务", back: "/more" },
  "/danger": { title: "危险操作", back: "/more" },
  "/core-config": { title: "核心配置", back: "/more" },
  "/apps-network": { title: "应用与网络", back: "/more" },
};

const mihomoOnlyPaths = new Set(["/groups", "/rules", "/providers", "/core-config"]);

function KsuUnavailablePage({ outdated = false }: { outdated?: boolean }) {
  return (
    <div className="flex min-h-dvh items-center justify-center px-6 py-12">
      <div className="w-full max-w-sm rounded-2xl border border-border bg-surface p-6 text-center shadow-sm">
        <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-2xl bg-warning-soft text-warning">
          <ShieldAlert size={24} />
        </div>
        <h1 className="mt-4 text-base font-semibold text-fg">{outdated ? "请升级 KernelSU" : "请从 KernelSU 打开"}</h1>
        <p className="mt-2 text-xs leading-5 text-fg-muted">
          {outdated
            ? "当前 KernelSU Manager 不支持异步命令接口 ksu.spawn()。请升级到 3.3 或更高版本后重新打开模块 WebUI。"
            : "KernelSU API 不可用。管理页需要通过 KernelSU 模块页的 WebUI 入口打开，直接访问文件不会获得 root 执行通道。"}
        </p>
      </div>
    </div>
  );
}

export function AppShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const toast = useToast();
  const ksuAvailable = isKernelsuAvailable();
  const asyncSupported = supportsAsyncExec();
  const ready = ksuAvailable && asyncSupported;
  const needsStatus = mihomoOnlyPaths.has(location.pathname);
  const statusQuery = useStatusQuery({ enabled: ready && needsStatus });
  const meta = pageMeta[location.pathname] || { title: "Box 管理" };

  useEffect(() => {
    window.scrollTo(0, 0);
  }, [location.pathname]);

  const onTaskFinished = useCallback(
    (task: { title: string; state: string }) => {
      if (task.state === "success") {
        toast({ title: `${task.title}完成`, tone: "success" });
      } else if (task.state === "failed" || task.state === "interrupted") {
        toast({ title: `${task.title}失败`, description: "打开任务中心查看详细输出", tone: "error" });
      } else if (task.state === "canceled") {
        toast({ title: `${task.title}已取消`, tone: "info" });
      }
    },
    [toast],
  );

  useTaskCompletionWatcher(onTaskFinished, ready);

  if (!ready) {
    return <KsuUnavailablePage outdated={ksuAvailable && !asyncSupported} />;
  }

  if (mihomoOnlyPaths.has(location.pathname)) {
    if (statusQuery.isLoading) return <PageSkeleton />;
    if (statusQuery.data && statusQuery.data.core !== "mihomo") {
      return (
        <MihomoUnsupportedPage
          title={meta.title}
          core={statusQuery.data.core}
          onBack={() => navigate("/more")}
        />
      );
    }
  }

  return (
    <div className="min-h-dvh bg-bg text-fg">
      <main className="mx-auto w-full max-w-3xl px-4 pb-28 pt-[calc(1rem+var(--safe-top))]">
        {meta.back ? (
          <div className="mb-2 flex items-center">
            <button
              type="button"
              onClick={() => navigate(meta.back as string)}
              className="-ml-2 flex h-10 w-10 items-center justify-center rounded-full text-fg-muted hover:bg-surface-muted hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
              aria-label="返回"
            >
              <ChevronLeft size={22} />
            </button>
          </div>
        ) : null}
        <Outlet />
      </main>
      <BottomNav />
    </div>
  );
}


function MihomoUnsupportedPage({ title, core, onBack }: { title: string; core: string; onBack: () => void }) {
  return (
    <div className="flex min-h-[70dvh] items-center justify-center px-2">
      <div className="w-full rounded-2xl border border-border bg-surface p-6 text-center shadow-sm">
        <ShieldAlert size={26} className="mx-auto text-warning" />
        <h1 className="mt-3 text-base font-semibold text-fg">{title} 仅支持 mihomo</h1>
        <p className="mt-2 text-xs leading-5 text-fg-muted">当前代理核心为 {core}。请先切回 mihomo，或到设置页修改代理核心。</p>
        <Button variant="secondary" className="mt-4" onClick={onBack}>返回更多</Button>
      </div>
    </div>
  );
}

export function RouteError() {
  const error = useRouteError();
  return (
    <div className="mx-auto max-w-md px-4 py-12">
      <div className="rounded-2xl border border-danger/20 bg-danger-soft p-5">
        <h1 className="text-base font-semibold text-danger">页面加载失败</h1>
        <p className="mt-2 text-xs text-fg">{getErrorMessage(error)}</p>
      </div>
    </div>
  );
}
