import { useEffect, useState } from "react";
import { Check, RefreshCw, Search } from "lucide-react";
import {
  listInstalledPackages,
  packageManagerAvailable,
} from "../../api/client";
import type { PackageInfo } from "../../api/types";
import { cn } from "../../lib/utils";
import { Button } from "./button";
import { Input } from "./input";
import { Sheet } from "./sheet";

const PACKAGE_NAME_RE = /^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$/;

type LoadStatus = "loading" | "ready" | "empty" | "unavailable" | "error";

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
  const [status, setStatus] = useState<LoadStatus>("unavailable");
  const [errorMessage, setErrorMessage] = useState("");
  const [query, setQuery] = useState("");
  const [manual, setManual] = useState("");
  const [manualError, setManualError] = useState("");
  const [retryKey, setRetryKey] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set(values));

  useEffect(() => {
    if (!open) return;
    setSelected(new Set(values));
    setQuery("");
    setManual("");
    setManualError("");
    setPackages([]);
    setErrorMessage("");
    setStatus("loading");

    if (!packageManagerAvailable()) {
      setStatus("unavailable");
      return;
    }

    let cancelled = false;
    void listInstalledPackages()
      .then((items) => {
        if (cancelled) return;
        setPackages(items);
        setStatus(items.length ? "ready" : "empty");
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setStatus("error");
        setErrorMessage(error instanceof Error ? error.message : "读取应用列表失败");
      });

    return () => {
      cancelled = true;
    };
  }, [open, retryKey]);

  const addManual = () => {
    const packageName = manual.trim();
    if (!packageName) return;
    if (!PACKAGE_NAME_RE.test(packageName)) {
      setManualError("请输入合法的 Android 包名，例如 com.example.app");
      return;
    }
    const next = new Set(selected);
    next.add(packageName);
    setSelected(next);
    setManual("");
    setManualError("");
  };

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
      <div className="space-y-3">
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            addManual();
          }}
        >
          <Input
            id="package-picker-manual"
            name="package-picker-manual"
            value={manual}
            onChange={(event) => { setManual(event.currentTarget.value); setManualError(""); }}
            placeholder="或手动输入包名：com.example.app"
            aria-label="手动输入包名"
            className="font-mono text-xs"
          />
          <Button type="submit" variant="secondary" disabled={!manual.trim()}>添加</Button>
        </form>
        {manualError ? <p className="text-[11px] text-danger">{manualError}</p> : null}

        {status === "loading" ? (
          <p className="rounded-xl bg-surface-muted px-3 py-3 text-xs text-fg-muted">正在读取应用列表…</p>
        ) : status === "unavailable" ? (
          <div className="rounded-xl bg-warning-soft p-3 text-xs leading-5 text-warning">
            当前管理器未提供包列表接口；可以手动输入包名，或更新 KernelSU Manager 后重试。
          </div>
        ) : status === "error" ? (
          <div className="space-y-2 rounded-xl bg-danger-soft p-3 text-xs leading-5 text-danger">
            <p>读取应用列表失败：{errorMessage}</p>
            <Button type="button" size="sm" variant="secondary" onClick={() => setRetryKey((value) => value + 1)}>
              <RefreshCw size={13} />重试
            </Button>
          </div>
        ) : status === "empty" ? (
          <div className="space-y-2 rounded-xl bg-surface-muted p-3 text-xs leading-5 text-fg-muted">
            <p>没有读取到应用；可以手动输入包名。</p>
            <Button type="button" size="sm" variant="secondary" onClick={() => setRetryKey((value) => value + 1)}>
              <RefreshCw size={13} />重试
            </Button>
          </div>
        ) : (
          <>
            <div className="relative">
              <Search size={15} className="absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle" />
              <Input id="package-picker-search" name="package-picker-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索应用名或包名" className="pl-9" />
            </div>
            <div className="max-h-[50vh] space-y-1 overflow-y-auto rounded-xl border border-border p-1">
              {filtered.length === 0 ? <p className="p-3 text-xs text-fg-muted">没有找到应用</p> : null}
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
            <p className="text-[10px] text-fg-subtle">共 {packages.length} 个应用</p>
          </>
        )}
      </div>
    </Sheet>
  );
}
