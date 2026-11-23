import { NavLink, Outlet } from "react-router-dom";
import { PageTitle } from "../../components/ui/page-title";
import { cn } from "../../lib/utils";

const tabClass = ({ isActive }: { isActive: boolean }) =>
  cn(
    "rounded-xl px-3 py-2 text-center text-xs font-medium transition-colors",
    isActive ? "bg-surface text-fg shadow-sm" : "text-fg-muted hover:text-fg",
  );

export function SubscriptionsPage() {
  return (
    <div className="space-y-4">
      <PageTitle title="订阅" description="节点订阅与规则策略分开管理，地址默认掩码显示" />
      <nav aria-label="订阅分类" className="grid grid-cols-2 gap-1 rounded-2xl border border-border bg-surface-muted p-1">
        <NavLink to="/subscriptions" end className={tabClass}>
          节点订阅
        </NavLink>
        <NavLink to="/subscriptions/rule-profile" className={tabClass}>
          规则与策略
        </NavLink>
      </nav>
      <Outlet />
    </div>
  );
}
