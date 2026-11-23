import { useState } from "react";
import { Link } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, ChevronRight, Database, ListTree, Pencil, RefreshCcw, Trash2 } from "lucide-react";
import { api, ApiError } from "../../api/client";
import { queryKeys, useRuleProfileQuery } from "../../api/queries";
import type { RuleProfileCounts, RuleProfileFetched } from "../../api/types";
import { ActionsMenu } from "../../components/ui/actions-menu";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card, CardBody } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input } from "../../components/ui/input";
import { Sheet } from "../../components/ui/sheet";
import { useToast } from "../../components/ui/toast";
import { formatDateTime, getErrorDetail, getErrorMessage } from "../../lib/utils";

const emptyCounts: RuleProfileCounts = { groups: 0, rule_providers: 0, rules: 0 };

function profileErrorText(error: unknown): string {
  const message = getErrorMessage(error);
  const detail = getErrorDetail(error).split("\n")[0].slice(0, 160);
  return detail ? `${message}（${detail}）` : message;
}

function CountLink({ to, label, count }: { to: string; label: string; count?: number }) {
  return (
    <Link to={to} className="flex items-center justify-between gap-2 rounded-xl border border-border bg-surface-muted/60 px-3 py-2.5 transition-colors hover:bg-surface-muted">
      <span className="whitespace-nowrap text-[11px] text-fg-muted">{label}</span>
      <span className="flex shrink-0 items-center gap-1 whitespace-nowrap text-xs font-semibold text-fg">
        {typeof count === "number" ? count : "编辑"}
        <ChevronRight size={13} className="text-fg-subtle" />
      </span>
    </Link>
  );
}

function CountsGrid({ counts }: { counts: RuleProfileCounts }) {
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
      <CountLink to="/groups" label="策略组" count={counts.groups} />
      <CountLink to="/rules" label="规则" count={counts.rules} />
      <CountLink to="/providers" label="规则集" count={counts.rule_providers} />
      <CountLink to="/core-config" label="配置文件" />
    </div>
  );
}

function CountSummary({ counts }: { counts: RuleProfileCounts }) {
  const items = [
    { label: "策略组", value: counts.groups },
    { label: "规则", value: counts.rules },
    { label: "规则集", value: counts.rule_providers },
  ];
  return (
    <div className="grid grid-cols-3 gap-2">
      {items.map((item) => (
        <div key={item.label} className="rounded-xl border border-border bg-surface-muted/60 px-3 py-2.5">
          <p className="text-[11px] text-fg-muted">{item.label}</p>
          <p className="mt-0.5 text-sm font-semibold text-fg">{item.value}</p>
        </div>
      ))}
    </div>
  );
}

export function RuleProfileCard() {
  const profileQuery = useRuleProfileQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [urlSheetOpen, setUrlSheetOpen] = useState(false);
  const [urlDraft, setUrlDraft] = useState("");
  const [urlLoading, setUrlLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [pending, setPending] = useState<RuleProfileFetched | null>(null);
  const [applying, setApplying] = useState<"save" | "reload" | null>(null);

  const profile = profileQuery.data;
  const counts = profile?.counts || emptyCounts;

  const refreshAll = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.ruleProfile }),
      queryClient.invalidateQueries({ queryKey: queryKeys.config }),
      queryClient.invalidateQueries({ queryKey: queryKeys.coreConfig }),
      queryClient.invalidateQueries({ queryKey: queryKeys.status }),
    ]);
  };

  const openEdit = async () => {
    setUrlSheetOpen(true);
    setUrlDraft("");
    if (!profile?.configured) return;
    setUrlLoading(true);
    try {
      const data = await api.ruleProfileUrl();
      setUrlDraft(data.url);
    } catch (error) {
      toast({ title: "读取规则订阅地址失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setUrlLoading(false);
    }
  };

  const saveUrl = async () => {
    const url = urlDraft.trim();
    if (!/^https?:\/\//i.test(url)) {
      toast({ title: "规则模板地址必须以 http:// 或 https:// 开头", tone: "error" });
      return;
    }
    setSaving(true);
    try {
      await api.setRuleProfile(url);
      toast({ title: "规则订阅地址已保存", tone: "success" });
      setUrlSheetOpen(false);
      await refreshAll();
    } catch (error) {
      toast({ title: "保存规则订阅地址失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const clearUrl = async () => {
    setClearing(true);
    try {
      await api.clearRuleProfile();
      toast({ title: "规则订阅地址已清除", description: "当前 config.yaml 不会改变", tone: "info" });
      setClearOpen(false);
      await refreshAll();
    } catch (error) {
      toast({ title: "清除规则订阅失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setClearing(false);
    }
  };

  const fetchTemplate = async () => {
    setFetching(true);
    try {
      const fetched = await api.fetchRuleProfile();
      setPending(fetched);
    } catch (error) {
      toast({ title: "下载或校验规则模板失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setFetching(false);
    }
  };

  const applyTemplate = async (reload: boolean) => {
    setApplying(reload ? "reload" : "save");
    try {
      const result = await api.applyRuleProfile(reload);
      toast({
        title: reload
          ? result.reloaded
            ? "规则与策略已保存并重载"
            : "规则与策略已保存，服务未运行"
          : "规则与策略已保存",
        description: reload && !result.reloaded ? "启动服务后生效" : undefined,
        tone: "success",
      });
      setPending(null);
      await refreshAll();
    } catch (error) {
      if (error instanceof ApiError && error.code === "E_RELOAD") {
        setPending(null);
        await refreshAll();
        toast({ title: "配置已保存，但重载失败", description: profileErrorText(error), tone: "error" });
      } else {
        toast({ title: "应用规则模板失败", description: profileErrorText(error), tone: "error" });
      }
    } finally {
      setApplying(null);
    }
  };

  if (profileQuery.isLoading) {
    return (
      <Card>
        <CardBody>
          <div className="mb-3 h-5 w-36 animate-pulse rounded bg-surface-muted" />
          <div className="h-20 animate-pulse rounded-xl bg-surface-muted" />
        </CardBody>
      </Card>
    );
  }

  if (profileQuery.error) {
    return (
      <Card>
        <CardBody>
          <ErrorState error={profileQuery.error} onRetry={() => void profileQuery.refetch()} />
        </CardBody>
      </Card>
    );
  }

  if (profile && !profile.supported) {
    return (
      <Card>
        <CardBody>
          <div className="flex items-center gap-2">
            <ListTree size={18} className="text-fg-muted" />
            <h2 className="text-sm font-semibold text-fg">规则与策略订阅</h2>
            <Badge tone="neutral">仅 mihomo</Badge>
          </div>
          <p className="mt-2 text-xs leading-5 text-fg-muted">当前核心不支持 mihomo 的 proxy-groups、rule-providers 和 rules 配置。</p>
        </CardBody>
      </Card>
    );
  }

  return (
    <>
      <Card>
        <CardBody>
          <div className="flex items-start gap-3">
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary-soft text-primary">
              <ListTree size={19} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="text-sm font-semibold text-fg">规则与策略订阅</h2>
                <Badge tone={profile?.configured ? "success" : "neutral"}>{profile?.configured ? "已配置" : "未配置"}</Badge>
              </div>
              <p className="mt-1 text-[11px] leading-5 text-fg-muted">独立于节点订阅，只更新策略组、规则集和规则；节点 provider 不受影响。</p>
            </div>
            {profile?.configured ? (
              <ActionsMenu
                items={[
                  { label: "修改地址", icon: <Pencil size={14} />, onSelect: () => void openEdit() },
                  { label: "清除订阅", icon: <Trash2 size={14} />, destructive: true, onSelect: () => setClearOpen(true) },
                ]}
              />
            ) : null}
          </div>

          {profile?.configured ? (
            <div className="mt-3 rounded-xl border border-border bg-surface-muted/60 px-3 py-2.5">
              <p className="truncate font-mono text-[11px] text-fg-muted">{profile.url_masked}</p>
              <p className="mt-1 text-[10px] text-fg-subtle">最近应用：{profile.updated_at ? formatDateTime(profile.updated_at) : "尚未应用"}</p>
            </div>
          ) : (
            <div className="mt-3 rounded-xl border border-dashed border-border px-3 py-3 text-xs leading-5 text-fg-muted">
              配置一个只包含策略组、规则集和规则的 mihomo 配置地址，即可一键更新这三段。
            </div>
          )}

          <div className="mt-3">
            <CountsGrid counts={counts} />
          </div>

          <div className="mt-3 flex flex-wrap gap-2">
            {profile?.configured ? (
              <>
                <Button variant="primary" onClick={() => void fetchTemplate()} loading={fetching} icon={<RefreshCcw size={15} />}>
                  更新规则与策略
                </Button>
                <Button variant="secondary" onClick={() => void openEdit()}>修改地址</Button>
              </>
            ) : (
              <Button variant="primary" onClick={() => void openEdit()}>配置规则订阅</Button>
            )}
          </div>
        </CardBody>
      </Card>

      <Sheet
        open={urlSheetOpen}
        onOpenChange={setUrlSheetOpen}
        title={profile?.configured ? "修改规则订阅地址" : "配置规则订阅"}
        description="只读取 proxy-groups、rule-providers、rules；不会导入 proxy-providers、DNS 或端口配置。"
        footer={
          <>
            <Button variant="secondary" disabled={saving} onClick={() => setUrlSheetOpen(false)}>取消</Button>
            <Button variant="primary" loading={saving} disabled={urlLoading} onClick={() => void saveUrl()}>保存</Button>
          </>
        }
      >
        <Field label="规则模板 URL" htmlFor="rule-profile-url" hint="支持返回 mihomo YAML 的 http/https 地址">
          <Input
            id="rule-profile-url"
            type="url"
            value={urlDraft}
            placeholder={urlLoading ? "读取中…" : "https://example.com/config.yaml"}
            readOnly={urlLoading}
            onInput={(event) => setUrlDraft(event.currentTarget.value)}
          />
        </Field>
      </Sheet>

      <ConfirmDialog
        open={clearOpen}
        onOpenChange={setClearOpen}
        title="清除规则订阅地址？"
        description="只会移除规则订阅地址和待应用快照，不会修改当前 config.yaml。"
        confirmLabel="清除"
        destructive
        loading={clearing}
        onConfirm={() => void clearUrl()}
      />

      <Sheet
        open={Boolean(pending)}
        onOpenChange={(open) => {
          if (!open && !applying) setPending(null);
        }}
        title="应用规则与策略订阅"
        description="三段整体替换，其他配置保持不变。"
        footer={
          <div className="flex w-full flex-wrap justify-end gap-2">
            <Button variant="ghost" disabled={Boolean(applying)} onClick={() => setPending(null)}>取消</Button>
            <Button variant="secondary" loading={applying === "save"} disabled={Boolean(applying)} onClick={() => void applyTemplate(false)}>仅保存</Button>
            <Button variant="primary" loading={applying === "reload"} disabled={Boolean(applying)} onClick={() => void applyTemplate(true)}>保存并重载</Button>
          </div>
        }
      >
        {pending ? (
          <div className="space-y-3">
            <CountSummary counts={pending.counts} />
            <div className="flex items-start gap-2 rounded-xl border border-warning/25 bg-warning-soft px-3 py-2.5 text-[11px] leading-5 text-warning">
              <AlertTriangle size={15} className="mt-0.5 shrink-0" />
              <span>将整体替换当前策略组、规则集和规则。配置文件会先备份为 config.yaml.bak。</span>
            </div>
            <div className="flex items-center gap-2 text-[10px] text-fg-subtle">
              <Database size={13} />
              <span>{pending.url_masked}</span>
            </div>
          </div>
        ) : null}
      </Sheet>
    </>
  );
}
