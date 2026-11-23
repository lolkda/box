import { Gauge, Layers3, ListTree, Menu, RadioTower } from "lucide-react";
import { NavLink, useLocation } from "react-router-dom";
import { cn } from "../../lib/utils";

const items = [
  { to: "/", label: "概览", icon: Gauge, end: true },
  { to: "/subscriptions", label: "订阅", icon: RadioTower, end: false },
  { to: "/groups", label: "策略", icon: Layers3, end: false },
  { to: "/rules", label: "规则", icon: ListTree, end: false },
  { to: "/more", label: "更多", icon: Menu, end: false },
];

const morePrefixes = ["/settings", "/providers", "/logs", "/tasks", "/danger", "/core-config", "/apps-network"];

export function BottomNav() {
  const location = useLocation();
  const moreActive = morePrefixes.some((prefix) => location.pathname === prefix || location.pathname.startsWith(`${prefix}/`));

  return (
    <nav
      className="fixed inset-x-0 bottom-0 z-40 border-t border-border/80 bg-surface/95 pb-[var(--safe-bottom)] backdrop-blur-xl"
      aria-label="主导航"
    >
      <div className="mx-auto grid h-[4.25rem] max-w-3xl grid-cols-5">
        {items.map((item) => {
          const active = item.to === "/more" ? moreActive : location.pathname === item.to || (!item.end && location.pathname.startsWith(`${item.to}/`));
          const Icon = item.icon;
          return (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end}
              className={cn(
                "relative flex min-h-11 flex-col items-center justify-center gap-0.5 px-1 text-[10px] font-medium transition-colors",
                active ? "text-primary" : "text-fg-muted hover:text-fg",
              )}
              aria-current={active ? "page" : undefined}
            >
              <span className={cn("flex h-7 w-12 items-center justify-center rounded-full transition-colors", active && "bg-primary-soft")}>
                <Icon size={20} strokeWidth={active ? 2.2 : 1.8} />
              </span>
              <span>{item.label}</span>
            </NavLink>
          );
        })}
      </div>
    </nav>
  );
}
