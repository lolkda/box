import { useState } from "react";
import { Link } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, ChevronRight, Database, ListTree, Pencil, Play, Plus, Power, RefreshCcw, Trash2 } from "lucide-react";
import { api, ApiError } from "../../api/client";
import { queryKeys, useRuleProfileListQuery } from "../../api/queries";
import type { RuleProfileCounts, RuleProfileItem } from "../../api/types";
import { ActionsMenu } from "../../components/ui/actions-menu";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card, CardBody } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { EmptyState } from "../../components/ui/empty-state";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input } from "../../components/ui/input";
import { PageSkeleton } from "../../components/ui/skeleton";
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

interface ProfileForm {
  name: string;
  url: string;
}

export function RuleProfileList() {
  const profilesQuery = useRuleProfileListQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<RuleProfileItem | null>(null);
  const [form, setForm] = useState<ProfileForm>({ name: "", url: "" });
  const [formLoading, setFormLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [updatingId, setUpdatingId] = useState<string | null>(null);
  const [useTarget, setUseTarget] = useState<RuleProfileItem | null>(null);
  const [applying, setApplying] = useState<"save" | "reload" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RuleProfileItem | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deactivateOpen, setDeactivateOpen] = useState(false);
  const [deactivating, setDeactivating] = useState(false);

  const refreshAll = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.ruleProfileList }),
      queryClient.invalidateQueries({ queryKey: queryKeys.ruleProfile }),
      queryClient.invalidateQueries({ queryKey: queryKeys.config }),
      queryClient.invalidateQueries({ queryKey: queryKeys.coreConfig }),
      queryClient.invalidateQueries({ queryKey: queryKeys.status }),
    ]);
  };

  const openAdd = () => {
    setEditing(null);
    setForm({ name: "", url: "" });
    setFormOpen(true);
  };

  const openEdit = (item: RuleProfileItem) => {
    setEditing(item);
    setForm({ name: item.name, url: "" });
    setFormOpen(true);
    setFormLoading(true);
    void api
      .ruleProfileUrl(item.id)
      .then((data) => setForm((current) => ({ ...current, url: data.url })))
      .catch((error) => toast({ title: "读取规则订阅地址失败", description: profileErrorText(error), tone: "error" }))
      .finally(() => setFormLoading(false));
  };

  const save = async () => {
    const name = form.name.trim();
    const url = form.url.trim();
    if (!name) {
      toast({ title: "规则订阅名称不能为空", tone: "error" });
      return;
    }
    if (!/^https?:\/\//i.test(url)) {
      toast({ title: "规则模板地址必须以 http:// 或 https:// 开头", tone: "error" });
      return;
    }
    setSaving(true);
    try {
      if (editing) {
        await api.editRuleProfile(editing.id, name, url);
        toast({ title: "规则订阅已更新", tone: "success" });
      } else {
        await api.addRuleProfile(name, url);
        toast({ title: "规则订阅已添加", description: "更新后才会生成待应用快照", tone: "success" });
      }
      setFormOpen(false);
      await refreshAll();
    } catch (error) {
      toast({ title: editing ? "更新规则订阅失败" : "添加规则订阅失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const updateOne = async (item: RuleProfileItem): Promise<RuleProfileItem | null> => {
    setUpdatingId(item.id);
    try {
      const updated = await api.updateRuleProfile(item.id);
      toast({ title: "规则模板已下载", description: "已进入待应用状态，点“使用”才会写入 config.yaml", tone: "success" });
      await refreshAll();
      return updated;
    } catch (error) {
      toast({ title: "下载或校验规则模板失败", description: profileErrorText(error), tone: "error" });
      return null;
    } finally {
      setUpdatingId(null);
    }
  };

  const beginUse = async (item: RuleProfileItem) => {
    if (item.pending) {
      setUseTarget(item);
      return;
    }
    const updated = await updateOne(item);
    if (updated) setUseTarget(updated);
  };

  const applyUse = async (reload: boolean) => {
    if (!useTarget) return;
    setApplying(reload ? "reload" : "save");
    try {
      const result = await api.useRuleProfile(useTarget.id, reload);
      toast({
        title: reload ? (result.reloaded ? "规则与策略已保存并重载" : "规则与策略已保存，服务未运行") : "规则与策略已保存",
        description: reload && !result.reloaded ? "启动服务后生效" : undefined,
        tone: "success",
      });
      setUseTarget(null);
      await refreshAll();
    } catch (error) {
      if (error instanceof ApiError && error.code === "E_RELOAD") {
        setUseTarget(null);
        await refreshAll();
        toast({ title: "配置已保存，但重载失败", description: profileErrorText(error), tone: "error" });
      } else {
        toast({ title: "应用规则模板失败", description: profileErrorText(error), tone: "error" });
      }
    } finally {
      setApplying(null);
    }
  };

  const remove = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await api.removeRuleProfile(deleteTarget.id);
      toast({ title: `已删除规则订阅：${deleteTarget.name}`, tone: "success" });
      setDeleteTarget(null);
      await refreshAll();
    } catch (error) {
      toast({ title: "删除规则订阅失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setDeleting(false);
    }
  };

  const deactivate = async () => {
    setDeactivating(true);
    try {
      await api.deactivateRuleProfile();
      toast({ title: "已停用规则订阅", description: "config.yaml 保持不变", tone: "info" });
      setDeactivateOpen(false);
      await refreshAll();
    } catch (error) {
      toast({ title: "停用规则订阅失败", description: profileErrorText(error), tone: "error" });
    } finally {
      setDeactivating(false);
    }
  };

  const profiles = profilesQuery.data?.items || [];
  const supported = profilesQuery.data?.supported ?? true;

  if (profilesQuery.isLoading) return <PageSkeleton />;

  if (profilesQuery.error) {
    return (
      <Card>
        <CardBody>
          <ErrorState error={profilesQuery.error} onRetry={() => void profilesQuery.refetch()} />
        </CardBody>
      </Card>
    );
  }

  if (!supported) {
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
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs leading-5 text-fg-muted">规则订阅只更新规则与策略，不影响节点</p>
        <Button variant="primary" size="icon" aria-label="添加规则订阅" onClick={openAdd}>
          <Plus size={18} />
        </Button>
      </div>

      {profiles.length === 0 ? (
        <EmptyState
          icon={<ListTree size={22} />}
          title="还没有规则订阅"
          description="添加一个只包含策略组、规则集和规则的 mihomo 配置地址。"
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>添加规则订阅</Button>}
        />
      ) : (
        <div className="space-y-3">
          {profiles.map((item) => {
            const counts = item.counts || emptyCounts;
            const updating = updatingId === item.id;
            return (
              <Card key={item.id}>
                <CardBody>
                  <div className="flex items-start gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <h2 className="truncate text-sm font-semibold text-fg">{item.name}</h2>
                        {item.active ? <Badge tone="success">使用中</Badge> : null}
                        {item.pending ? <Badge tone="warning">待应用</Badge> : null}
                        {!item.active && !item.pending ? <Badge tone="neutral">未使用</Badge> : null}
                      </div>
                      <p className="mt-1 truncate font-mono text-[11px] text-fg-muted">{item.url_masked}</p>
                      <p className="mt-1 text-[10px] text-fg-subtle">
                        下载：{item.fetched_at ? formatDateTime(item.fetched_at) : item.pending ? "已暂存" : "尚未下载"} · 应用：{item.applied_at ? formatDateTime(item.applied_at) : "尚未应用"}
                      </p>
                    </div>
                    <ActionsMenu
                      items={[
                        ...(item.active
                          ? item.pending
                            ? [{ label: "应用此更新", icon: <Play size={14} />, onSelect: () => setUseTarget(item) }]
                            : []
                          : [{ label: "使用", icon: <Play size={14} />, onSelect: () => void beginUse(item) }]),
                        { label: "更新", icon: <RefreshCcw size={14} />, onSelect: () => void updateOne(item), disabled: updating },
                        { label: "编辑", icon: <Pencil size={14} />, onSelect: () => openEdit(item) },
                        ...(item.active ? [{ label: "停用", icon: <Power size={14} />, onSelect: () => setDeactivateOpen(true) }] : []),
                        {
                          label: "删除",
                          icon: <Trash2 size={14} />,
                          destructive: true,
                          disabled: !item.can_delete,
                          onSelect: () => setDeleteTarget(item),
                        },
                      ]}
                    />
                  </div>

                  <div className="mt-3">
                    <CountsGrid counts={counts} />
                  </div>
                </CardBody>
              </Card>
            );
          })}
        </div>
      )}

      <Sheet
        open={formOpen}
        onOpenChange={setFormOpen}
        title={editing ? "编辑规则订阅" : "添加规则订阅"}
        description="只读取 proxy-groups、rule-providers、rules；不会导入 proxy-providers、DNS 或端口配置。"
        footer={
          <>
            <Button variant="secondary" disabled={saving} onClick={() => setFormOpen(false)}>取消</Button>
            <Button variant="primary" loading={saving} disabled={formLoading} onClick={() => void save()}>保存</Button>
          </>
        }
      >
        <div className="space-y-4">
          <Field label="名称" htmlFor="rule-profile-name" hint="同一时间只能使用一条；名称不能重复">
            <Input
              id="rule-profile-name"
              value={form.name}
              placeholder="例如：机场 A 规则"
              onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, name: value })); }}
            />
          </Field>
          <Field label="规则模板 URL" htmlFor="rule-profile-url" hint="支持返回 mihomo YAML 的 http/https 地址">
            <Input
              id="rule-profile-url"
              type="url"
              value={form.url}
              placeholder={formLoading ? "读取中…" : "https://example.com/config.yaml"}
              readOnly={formLoading}
              onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, url: value })); }}
            />
          </Field>
          <p className="rounded-xl bg-surface-muted px-3 py-2 text-[11px] leading-5 text-fg-muted">
            更新后只生成待应用快照；点“使用”并确认后才会替换 config.yaml 的三段。
          </p>
        </div>
      </Sheet>

      <Sheet
        open={Boolean(useTarget)}
        onOpenChange={(open) => { if (!open && !applying) setUseTarget(null); }}
        title="应用规则与策略订阅"
        description="只应用选中这一条，不会合并其他规则订阅。"
        footer={
          <div className="flex w-full flex-wrap justify-end gap-2">
            <Button variant="ghost" disabled={Boolean(applying)} onClick={() => setUseTarget(null)}>取消</Button>
            <Button variant="secondary" loading={applying === "save"} disabled={Boolean(applying)} onClick={() => void applyUse(false)}>仅保存</Button>
            <Button variant="primary" loading={applying === "reload"} disabled={Boolean(applying)} onClick={() => void applyUse(true)}>保存并重载</Button>
          </div>
        }
      >
        {useTarget ? (
          <div className="space-y-3">
            <p className="text-xs font-medium text-fg">{useTarget.name}</p>
            <CountSummary counts={useTarget.counts} />
            <div className="flex items-start gap-2 rounded-xl border border-warning/25 bg-warning-soft px-3 py-2.5 text-[11px] leading-5 text-warning">
              <AlertTriangle size={15} className="mt-0.5 shrink-0" />
              <span>将整体替换当前策略组、规则集和规则。配置文件会先备份为 config.yaml.bak。</span>
            </div>
            <div className="flex items-center gap-2 text-[10px] text-fg-subtle">
              <Database size={13} />
              <span>{useTarget.url_masked}</span>
            </div>
          </div>
        ) : null}
      </Sheet>

      <ConfirmDialog
        open={Boolean(deleteTarget)}
        onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
        title={`删除规则订阅「${deleteTarget?.name || ""}」？`}
        description="会删除该条目的 URL、待应用快照和计数，不会修改 config.yaml。"
        confirmLabel="删除"
        destructive
        loading={deleting}
        onConfirm={() => void remove()}
      />

      <ConfirmDialog
        open={deactivateOpen}
        onOpenChange={setDeactivateOpen}
        title="停用规则订阅？"
        description="只清除“当前使用”标记，不会修改 config.yaml；之后可以删除该条目。"
        confirmLabel="停用"
        loading={deactivating}
        onConfirm={() => void deactivate()}
      />
    </>
  );
}
