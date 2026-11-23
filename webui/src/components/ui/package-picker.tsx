import { useEffect, useState } from "react";
import { Check, Search } from "lucide-react";
import { listInstalledPackages, packageManagerAvailable } from "../../api/client";
import type { PackageInfo } from "../../api/types";
import { cn } from "../../lib/utils";
import { Button } from "./button";
import { Input } from "./input";
import { Sheet } from "./sheet";

export function PackagePicker({
  open,
  onOpenChange,
  values,
  onChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  values: string[];
  onChange: (values: string[]) => void;
}) {
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set(values));
  const available = packageManagerAvailable();

  useEffect(() => {
    if (!open) return;
    setSelected(new Set(values));
    setQuery("");
    if (!available) return;
    setLoading(true);
    void listInstalledPackages()
      .then(setPackages)
      .finally(() => setLoading(false));
  }, [open, values, available]);

  const filtered = packages.filter((item) => {
    const term = query.trim().toLowerCase();
    return !term || item.packageName.toLowerCase().includes(term) || item.appLabel.toLowerCase().includes(term);
  });

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="选择应用"
      description="选择需要纳入清单的应用包名。"
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>取消</Button>
          <Button variant="primary" onClick={() => { onChange([...selected]); onOpenChange(false); }}>
            应用选择（{selected.size}）
          </Button>
        </>
      }
    >
      {!available ? (
        <div className="rounded-xl bg-warning-soft p-3 text-xs leading-5 text-warning">
          当前 KernelSU 管理器未提供应用列表 API，请手动填写包名。
        </div>
      ) : (
        <div className="space-y-3">
          <div className="relative">
            <Search size={15} className="absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle" />
            <Input id="package-picker-search" name="package-picker-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索应用名或包名" className="pl-9" />
          </div>
          <div className="max-h-[50vh] space-y-1 overflow-y-auto rounded-xl border border-border p-1">
            {loading ? <p className="p-3 text-xs text-fg-muted">正在读取应用列表…</p> : null}
            {!loading && filtered.length === 0 ? <p className="p-3 text-xs text-fg-muted">没有找到应用</p> : null}
            {filtered.map((item) => {
              const checked = selected.has(item.packageName);
              return (
                <button
                  key={item.packageName}
                  type="button"
                  onClick={() => {
                    const next = new Set(selected);
                    if (checked) next.delete(item.packageName);
                    else next.add(item.packageName);
                    setSelected(next);
                  }}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left",
                    checked ? "bg-primary-soft/60" : "hover:bg-surface-muted",
                  )}
                >
                  <span className={cn("flex h-5 w-5 shrink-0 items-center justify-center rounded-md border", checked ? "border-primary bg-primary text-primary-fg" : "border-border-strong")}>
                    {checked ? <Check size={12} /> : null}
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-xs font-medium text-fg">{item.appLabel}</span>
                    <span className="block truncate font-mono text-[10px] text-fg-muted">{item.packageName}</span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </Sheet>
  );
}
