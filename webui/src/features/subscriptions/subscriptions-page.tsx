import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { DownloadCloud, Pencil, Plus, RefreshCcw, RadioTower, Trash2 } from "lucide-react";
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
import { PageTitle } from "../../components/ui/page-title";
import { Sheet } from "../../components/ui/sheet";
import { PageSkeleton } from "../../components/ui/skeleton";
import { useToast } from "../../components/ui/toast";
import { formatBytes, formatDateTime, getErrorMessage } from "../../lib/utils";

interface SubscriptionForm {
  name: string;
  url: string;
}

export function SubscriptionsPage() {
  const subscriptionsQuery = useSubscriptionsQuery();
  const startTask = useTaskStart();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Subscription | null>(null);
  const [form, setForm] = useState<SubscriptionForm>({ name: "", url: "" });
  const [detailLoading, setDetailLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<Subscription | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const [updatingAll, setUpdatingAll] = useState(false);

  useEffect(() => {
    if (!formOpen) return;
    if (!editing) {
      setForm({ name: "", url: "" });
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    void api
      .subscription(editing.name)
      .then((detail) => {
        if (!cancelled) setForm({ name: detail.name, url: detail.url });
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
    setForm({ name: "", url: "" });
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
    setSaving(true);
    try {
      if (editing) {
        await api.setSubscription(editing.name, name || editing.name, url);
        toast({ title: "订阅已更新", tone: "success" });
      } else {
        await api.addSubscription(url, name);
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
      <PageTitle
        title="订阅"
        description="订阅地址默认掩码显示，只有编辑时读取完整地址"
        action={
          <>
            <Button variant="secondary" size="icon" aria-label="更新全部订阅" onClick={() => void updateAll()} loading={updatingAll}>
              <DownloadCloud size={17} />
            </Button>
            <Button variant="primary" size="icon" aria-label="添加订阅" onClick={openAdd}>
              <Plus size={18} />
            </Button>
          </>
        }
      />

      {subscriptionsQuery.isLoading ? (
        <PageSkeleton />
      ) : subscriptionsQuery.error ? (
        <ErrorState error={subscriptionsQuery.error} onRetry={() => void subscriptionsQuery.refetch()} />
      ) : items.length === 0 ? (
        <EmptyState
          title="还没有订阅"
          description="添加一个订阅地址后，Box 会拉取节点并生成 provider 文件。"
          icon={<RadioTower size={22} />}
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>添加订阅</Button>}
        />
      ) : (
        <div className="space-y-3">
          {items.map((subscription) => (
            <Card key={subscription.name} className="overflow-hidden">
              <div className="p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h2 className="truncate text-[15px] font-semibold text-fg">{subscription.name}</h2>
                      <Badge tone={subscription.provider === "downloaded" || subscription.provider === "aggregated" ? "success" : "warning"}>
                        {subscription.provider === "aggregated"
                          ? `${subscription.provider_count} 个 provider`
                          : subscription.provider === "downloaded" ? "已下载" : "未下载"}
                      </Badge>
                    </div>
                    <p className="mt-1 truncate font-mono text-[11px] text-fg-muted">{subscription.url_masked}</p>
                  </div>
                  <ActionsMenu
                    items={[
                      { label: "立即更新", icon: <RefreshCcw size={14} />, onSelect: () => void updateOne(subscription) },
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
        title={editing ? "编辑订阅" : "添加订阅"}
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
          {detailLoading ? <p className="text-xs text-fg-muted">正在读取订阅详情…</p> : null}
          <p className="rounded-xl bg-surface-muted px-3 py-2 text-[11px] leading-5 text-fg-muted">
            订阅 URL 可能包含访问令牌。列表只显示域名，完整地址仅在编辑时读取。
          </p>
        </div>
      </Sheet>

      <ConfirmDialog
        open={Boolean(deleting)}
        onOpenChange={(open) => { if (!open) setDeleting(null); }}
        title={`删除订阅「${deleting?.name || ""}」？`}
        description="会从 subscriptions.list 删除该订阅，并删除对应的 provider 文件。此操作不可撤销。"
        confirmLabel="删除"
        destructive
        loading={deleteLoading}
        onConfirm={() => void remove()}
      />
    </div>
  );
}
