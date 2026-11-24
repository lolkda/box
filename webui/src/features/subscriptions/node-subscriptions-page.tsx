import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { DownloadCloud, Pencil, Plus, Power, RefreshCcw, RadioTower, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useSubscriptionsQuery, useTaskStart } from "../../api/queries";
import type { Subscription } from "../../api/types";
import { ActionsMenu } from "../../components/ui/actions-menu";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { EmptyState } from "../../components/ui/empty-state";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input } from "../../components/ui/input";
import { Sheet } from "../../components/ui/sheet";
import { PageSkeleton } from "../../components/ui/skeleton";
import { useToast } from "../../components/ui/toast";
import { cn, formatBytes, formatDateTime, getErrorMessage } from "../../lib/utils";
import { isValidUserAgent, userAgentPresets } from "../../lib/user-agent";

interface SubscriptionForm {
  name: string;
  url: string;
  userAgent: string;
}

export function NodeSubscriptionsPage() {
  const subscriptionsQuery = useSubscriptionsQuery();
  const startTask = useTaskStart();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Subscription | null>(null);
  const [form, setForm] = useState<SubscriptionForm>({ name: "", url: "", userAgent: "" });
  const [detailLoading, setDetailLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<Subscription | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const [toggleTarget, setToggleTarget] = useState<Subscription | null>(null);
  const [toggling, setToggling] = useState(false);
  const [updatingAll, setUpdatingAll] = useState(false);

  useEffect(() => {
    if (!formOpen) return;
    if (!editing) {
      setForm({ name: "", url: "", userAgent: "" });
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    void api
      .subscription(editing.name)
      .then((detail) => {
        if (!cancelled) setForm({ name: detail.name, url: detail.url, userAgent: detail.user_agent || "" });
      })
      .catch((error) => {
        if (!cancelled) toast({ title: "读取订阅详情失败", description: getErrorMessage(error), tone: "error" });
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [editing, formOpen, toast]);

  const openAdd = () => {
    setEditing(null);
    setForm({ name: "", url: "", userAgent: "" });
    setFormOpen(true);
  };

  const openEdit = (subscription: Subscription) => {
    setEditing(subscription);
    setFormOpen(true);
  };

  const save = async () => {
    const url = form.url.trim();
    const name = form.name.trim();
    if (!/^https?:\/\//i.test(url)) {
      toast({ title: "订阅地址必须以 http:// 或 https:// 开头", tone: "error" });
      return;
    }
    if (name.includes("|")) {
      toast({ title: "订阅名字不能包含 |", tone: "error" });
      return;
    }
    const userAgent = form.userAgent;
    if (!isValidUserAgent(userAgent, true)) {
      toast({ title: "User-Agent 不能为空、不能包含换行或控制字符，最多 255 个字符", tone: "error" });
      return;
    }
    setSaving(true);
    try {
      if (editing) {
        await api.setSubscription(editing.name, name || editing.name, url, userAgent);
        toast({ title: "订阅已更新", tone: "success" });
      } else {
        await api.addSubscription(url, name, userAgent);
        toast({ title: "订阅已添加", description: "可在任务中心更新全部订阅", tone: "success" });
      }
      setFormOpen(false);
      await queryClient.invalidateQueries({ queryKey: queryKeys.subscriptions });
    } catch (error) {
      toast({ title: editing ? "更新订阅失败" : "添加订阅失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!deleting) return;
    setDeleteLoading(true);
    try {
      await api.removeSubscription(deleting.name);
      toast({ title: `已删除订阅：${deleting.name}`, tone: "success" });
      setDeleting(null);
      await queryClient.invalidateQueries({ queryKey: queryKeys.subscriptions });
    } catch (error) {
      toast({ title: "删除订阅失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setDeleteLoading(false);
    }
  };

  const updateOne = async (subscription: Subscription) => {
    try {
      await startTask("sub-update", subscription.name);
      toast({ title: "已开始更新订阅", description: subscription.name, tone: "info" });
      navigate("/tasks");
    } catch (error) {
      toast({ title: "启动更新失败", description: getErrorMessage(error), tone: "error" });
    }
  };

  const toggleEnabled = async () => {
    if (!toggleTarget) return;
    const next = !toggleTarget.enabled;
    setToggling(true);
    try {
      const result = await api.setSubscriptionEnabled(toggleTarget.name, next);
      toast({
        title: next ? "已启用节点订阅" : "已禁用节点订阅",
        description: next
          ? result.missing ? "节点文件不存在，请点“立即更新”拉取" : "节点已恢复到 proxy-providers"
          : "节点已移出 proxy-providers，URL 和本地文件保留",
        tone: "success",
      });
      setToggleTarget(null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.subscriptions }),
        queryClient.invalidateQueries({ queryKey: queryKeys.config }),
        queryClient.invalidateQueries({ queryKey: queryKeys.status }),
      ]);
    } catch (error) {
      toast({ title: next ? "启用订阅失败" : "禁用订阅失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setToggling(false);
    }
  };

  const updateAll = async () => {
    setUpdatingAll(true);
    try {
      await startTask("sub-update");
      toast({ title: "已开始更新全部订阅", tone: "info" });
      navigate("/tasks");
    } catch (error) {
      toast({ title: "启动更新失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setUpdatingAll(false);
    }
  };

  const items = subscriptionsQuery.data || [];

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-fg-muted">节点订阅只更新节点，不影响规则与策略</p>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="secondary" size="icon" aria-label="更新全部订阅" onClick={() => void updateAll()} loading={updatingAll}>
            <DownloadCloud size={17} />
          </Button>
          <Button variant="primary" size="icon" aria-label="添加订阅" onClick={openAdd}>
            <Plus size={18} />
          </Button>
        </div>
      </div>

      {subscriptionsQuery.isLoading ? (
        <PageSkeleton />
      ) : subscriptionsQuery.error ? (
        <ErrorState error={subscriptionsQuery.error} onRetry={() => void subscriptionsQuery.refetch()} />
      ) : items.length === 0 ? (
        <EmptyState
          title="还没有节点订阅"
          description="添加一个订阅地址后，Box 会拉取节点并生成 provider 文件。"
          icon={<RadioTower size={22} />}
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>添加订阅</Button>}
        />
      ) : (
        <div className="space-y-3">
          {items.map((subscription) => (
            <Card key={subscription.name} className={cn("overflow-hidden", !subscription.enabled && "opacity-70")}>
              <div className="p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h2 className="truncate text-[15px] font-semibold text-fg">{subscription.name}</h2>
                      {subscription.enabled === false ? (
                        <Badge tone="neutral">已禁用</Badge>
                      ) : (
                        <Badge tone={subscription.provider === "downloaded" || subscription.provider === "aggregated" ? "success" : "warning"}>
                          {subscription.provider === "aggregated"
                            ? `${subscription.provider_count} 个 provider`
                            : subscription.provider === "downloaded" ? "已下载" : "未下载"}
                        </Badge>
                      )}
                    </div>
                    <p className="mt-1 truncate font-mono text-[11px] text-fg-muted">{subscription.url_masked}</p>
                  </div>
                  <ActionsMenu
                    items={[
                      ...(subscription.enabled !== false ? [{ label: "立即更新", icon: <RefreshCcw size={14} />, onSelect: () => void updateOne(subscription) }] : []),
                      {
                        label: subscription.enabled === false ? "启用订阅" : "禁用订阅",
                        icon: <Power size={14} />,
                        onSelect: () => setToggleTarget(subscription),
                      },
                      { label: "编辑", icon: <Pencil size={14} />, onSelect: () => openEdit(subscription) },
                      { label: "删除", icon: <Trash2 size={14} />, destructive: true, onSelect: () => setDeleting(subscription) },
                    ]}
                  />
                </div>
                <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-fg-subtle">
                  <span>文件 {formatBytes(subscription.provider_size)}</span>
                  {subscription.provider_count > 0 && <span>派生 {subscription.provider_count} 个 provider</span>}
                  <span>更新 {formatDateTime(subscription.updated_at)}</span>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <Sheet
        open={formOpen}
        onOpenChange={setFormOpen}
        title={editing ? "编辑节点订阅" : "添加节点订阅"}
        description={editing ? "修改订阅名或地址后立即生效。" : "名字留空时将从域名自动生成。"}
        footer={
          <>
            <Button variant="secondary" onClick={() => setFormOpen(false)} disabled={saving}>取消</Button>
            <Button variant="primary" onClick={() => void save()} loading={saving} disabled={detailLoading}>
              {editing ? "保存" : "添加"}
            </Button>
          </>
        }
      >
        <div className="space-y-4">
          <Field label="名字" htmlFor="subscription-name" hint="不能包含 | 字符">
            <Input
              id="subscription-name"
              value={form.name}
              placeholder="例如：机场 A"
              onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, name: value })); }}
            />
          </Field>
          <Field label="订阅地址" htmlFor="subscription-url" required hint="支持 http:// 或 https://">
            <Input
              id="subscription-url"
              value={form.url}
              placeholder="https://example.com/sub"
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, url: value })); }}
            />
          </Field>
          <Field label="User-Agent（可选）" htmlFor="subscription-user-agent" hint="留空继承全局；修改后下次更新订阅生效">
            <datalist id="subscription-user-agent-presets">
              {userAgentPresets.map((preset) => <option value={preset} key={preset} />)}
            </datalist>
            <Input
              id="subscription-user-agent"
              list="subscription-user-agent-presets"
              value={form.userAgent}
              placeholder="clash-verge/v2.4.5"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              className="font-mono text-xs"
              onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, userAgent: value })); }}
            />
          </Field>
          {detailLoading ? <p className="text-xs text-fg-muted">正在读取订阅详情…</p> : null}
          <p className="rounded-xl bg-surface-muted px-3 py-2 text-[11px] leading-5 text-fg-muted">
            订阅 URL 可能包含访问令牌。列表只显示域名，完整地址仅在编辑时读取。
          </p>
        </div>
      </Sheet>

      <ConfirmDialog
        open={Boolean(toggleTarget)}
        onOpenChange={(open) => { if (!open && !toggling) setToggleTarget(null); }}
        title={toggleTarget?.enabled === false ? `启用节点订阅「${toggleTarget?.name || ""}」？` : `禁用节点订阅「${toggleTarget?.name || ""}」？`}
        description={toggleTarget?.enabled === false
          ? "会把该订阅重新加入 proxy-providers；如果本地节点文件不存在，请启用后点“立即更新”。"
          : "会从 proxy-providers 移除该订阅及其派生 provider，节点不再参与策略组；URL 和本地文件保留。当前策略组显式引用该 provider 时会拒绝禁用。"}
        confirmLabel={toggleTarget?.enabled === false ? "启用" : "禁用"}
        loading={toggling}
        onConfirm={() => void toggleEnabled()}
      />

      <ConfirmDialog
        open={Boolean(deleting)}
        onOpenChange={(open) => { if (!open) setDeleting(null); }}
        title={`删除节点订阅「${deleting?.name || ""}」？`}
        description="会从 subscriptions.list 删除该节点订阅，并删除对应的 provider 文件。此操作不可撤销。"
        confirmLabel="删除"
        destructive
        loading={deleteLoading}
        onConfirm={() => void remove()}
      />
    </div>
  );
}
