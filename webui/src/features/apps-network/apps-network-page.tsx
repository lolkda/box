import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Network, PackageSearch, Plus, Save, ShieldCheck, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useAppListsQuery } from "../../api/queries";
import type { PackageEntry } from "../../api/types";
import { StickyActionBar } from "../../components/layout/sticky-action-bar";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card, CardBody, CardHeader } from "../../components/ui/card";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input } from "../../components/ui/input";
import { PageTitle } from "../../components/ui/page-title";
import { PackagePicker } from "../../components/ui/package-picker";
import { PageSkeleton } from "../../components/ui/skeleton";
import { TagInput } from "../../components/ui/tag-input";
import { useToast } from "../../components/ui/toast";
import { cn, getErrorMessage } from "../../lib/utils";

type Tab = "package" | "gid" | "interfaces";

export function AppsNetworkPage() {
  const listsQuery = useAppListsQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [tab, setTab] = useState<Tab>("package");
  const [packageEntries, setPackageEntries] = useState<PackageEntry[]>([]);
  const [gid, setGid] = useState<string[]>([]);
  const [interfaces, setInterfaces] = useState<{ allow: string[]; ignore: string[] }>({ allow: [], ignore: [] });
  const [uidDraft, setUidDraft] = useState("0");
  const [packageDraft, setPackageDraft] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!listsQuery.data) return;
    setPackageEntries(listsQuery.data.package.map((entry) => ({ ...entry })));
    setGid([...listsQuery.data.gid]);
    setInterfaces({ allow: [...listsQuery.data.interfaces.allow], ignore: [...listsQuery.data.interfaces.ignore] });
  }, [listsQuery.data]);

  const original = listsQuery.data;
  const dirty = Boolean(original && (
    JSON.stringify(packageEntries) !== JSON.stringify(original.package) ||
    JSON.stringify(gid) !== JSON.stringify(original.gid) ||
    JSON.stringify(interfaces) !== JSON.stringify(original.interfaces)
  ));

  const addPackage = () => {
    const packageName = packageDraft.trim();
    if (!packageName) return;
    if (packageEntries.some((entry) => entry.package === packageName && (entry.uid || "") === uidDraft.trim())) return;
    setPackageEntries((current) => [...current, { uid: uidDraft.trim() || "0", package: packageName }]);
    setPackageDraft("");
  };

  const addPickedPackages = (packages: string[]) => {
    setPackageEntries((current) => {
      const next = [...current];
      for (const packageName of packages) {
        if (!next.some((entry) => entry.package === packageName)) next.push({ uid: "0", package: packageName });
      }
      return next;
    });
  };

  const save = async () => {
    setSaving(true);
    try {
      await api.applyAppLists({
        package: packageEntries.map((entry) => ({ uid: entry.uid || "", package: entry.package })),
        gid,
        interfaces,
      });
      toast({ title: "应用与网络清单已保存", tone: "success" });
      await queryClient.invalidateQueries({ queryKey: queryKeys.appLists });
    } catch (error) {
      toast({ title: "保存清单失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const tabs = useMemo(() => [
    { id: "package" as const, label: "应用包", icon: <PackageSearch size={14} /> },
    { id: "gid" as const, label: "GID", icon: <ShieldCheck size={14} /> },
    { id: "interfaces" as const, label: "网络接口", icon: <Network size={14} /> },
  ], []);

  if (listsQuery.isLoading) return <PageSkeleton />;
  if (listsQuery.error) return <ErrorState error={listsQuery.error} onRetry={() => void listsQuery.refetch()} />;

  return (
    <div className="space-y-4 pb-8">
      <PageTitle
        title="应用与网络"
        description="管理 proxy 黑白名单的应用、GID 与网络接口清单"
        action={<Badge tone={dirty ? "warning" : "neutral"}>{dirty ? "未保存" : "已同步"}</Badge>}
      />

      <div className="grid grid-cols-3 rounded-xl bg-surface-muted p-1">
        {tabs.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => setTab(item.id)}
            className={cn("flex h-9 items-center justify-center gap-1 rounded-lg text-xs font-medium transition-colors", tab === item.id ? "bg-surface text-fg shadow-sm" : "text-fg-muted")}
          >
            {item.icon} {item.label}
          </button>
        ))}
      </div>

      {tab === "package" ? (
        <Card>
          <CardHeader title="应用包清单" description="默认 UID 0（主用户）；工作资料可填写 10、999 等用户 ID。" />
          <CardBody className="space-y-3">
            <div className="flex gap-2">
              <Input value={uidDraft} onChange={(event) => setUidDraft(event.target.value)} className="w-16 text-center font-mono text-xs" aria-label="用户 ID" />
              <Input value={packageDraft} onChange={(event) => setPackageDraft(event.target.value)} placeholder="com.example.app" className="font-mono text-xs" aria-label="应用包名" />
              <Button size="icon" variant="secondary" onClick={addPackage} aria-label="添加应用"><Plus size={17} /></Button>
              <Button size="icon" variant="secondary" onClick={() => setPickerOpen(true)} aria-label="选择已安装应用"><PackageSearch size={17} /></Button>
            </div>
            <div className="space-y-2">
              {packageEntries.length === 0 ? (
                <p className="rounded-xl bg-surface-muted px-3 py-3 text-xs text-fg-muted">还没有应用条目；添加后会写入 package.list.cfg。</p>
              ) : packageEntries.map((entry, index) => (
                <div key={`${entry.package}-${index}`} className="flex items-center gap-2 rounded-xl border border-border px-3 py-2">
                  <Input
                    value={entry.uid || ""}
                    onChange={(event) => {
                      const value = event.currentTarget.value;
                      setPackageEntries((current) => current.map((item, itemIndex) => itemIndex === index ? { ...item, uid: value } : item));
                    }}
                    className="w-16 text-center font-mono text-xs"
                    aria-label={`${entry.package} 的用户 ID`}
                  />
                  <span className="min-w-0 flex-1 truncate font-mono text-xs text-fg">{entry.package}</span>
                  <button
                    type="button"
                    onClick={() => setPackageEntries((current) => current.filter((_, itemIndex) => itemIndex !== index))}
                    className="rounded-lg p-1.5 text-fg-muted hover:bg-danger-soft hover:text-danger"
                    aria-label={`移除 ${entry.package}`}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              ))}
            </div>
            <p className="text-[11px] leading-5 text-fg-subtle">应用包清单用于 proxy_mode 黑名单/白名单；UID:包名 格式可覆盖工作资料。</p>
          </CardBody>
        </Card>
      ) : null}

      {tab === "gid" ? (
        <Card>
          <CardHeader title="GID 清单" description="每行一个 GID；用于按用户组匹配流量。" />
          <CardBody>
            <TagInput id="gid-list" values={gid} onChange={setGid} placeholder="例如 1000" mono />
          </CardBody>
        </Card>
      ) : null}

      {tab === "interfaces" ? (
        <Card>
          <CardHeader title="网络接口" description="allow 是允许接入的网络接口，ignore 是需要忽略的接口。" />
          <CardBody className="space-y-4">
            <Field label="允许接口" htmlFor="allow-interfaces" hint="支持接口前缀，例如 wlan+、ap+、rndis+">
              <TagInput id="allow-interfaces" values={interfaces.allow} onChange={(allow) => setInterfaces((current) => ({ ...current, allow }))} placeholder="例如 wlan+" mono />
            </Field>
            <Field label="忽略接口" htmlFor="ignore-interfaces" hint="例如 wlan0">
              <TagInput id="ignore-interfaces" values={interfaces.ignore} onChange={(ignore) => setInterfaces((current) => ({ ...current, ignore }))} placeholder="例如 wlan0" mono />
            </Field>
          </CardBody>
        </Card>
      ) : null}

      <StickyActionBar
        label="应用与网络操作栏"
        status={dirty ? "有未保存的清单修改" : "清单已同步"}
      >
        {dirty ? (
          <Button
            variant="secondary"
            size="md"
            className="h-10 flex-1 px-3 text-xs sm:flex-none"
            disabled={saving}
            onClick={() => {
              if (!original) return;
              setPackageEntries(original.package.map((entry) => ({ ...entry })));
              setGid([...original.gid]);
              setInterfaces({ allow: [...original.interfaces.allow], ignore: [...original.interfaces.ignore] });
            }}
          >
            撤销
          </Button>
        ) : null}
        <Button
          variant="primary"
          size="md"
          className={cn("h-10 px-3 text-xs", dirty ? "flex-1 sm:flex-none" : "w-full sm:w-auto")}
          onClick={() => void save()}
          loading={saving}
          disabled={!dirty}
          icon={<Save size={15} />}
        >
          保存清单
        </Button>
      </StickyActionBar>

      <PackagePicker
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        values={packageEntries.map((entry) => entry.package)}
        onChange={(values) => addPickedPackages(values)}
      />
    </div>
  );
}
