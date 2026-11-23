import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Layers3, Pencil, Plus, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useConfigQuery } from "../../api/queries";
import type { ProxyGroup } from "../../api/types";
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
import { Select } from "../../components/ui/select";
import { Sheet } from "../../components/ui/sheet";
import { PageSkeleton } from "../../components/ui/skeleton";
import { Switch } from "../../components/ui/switch";
import { TagInput } from "../../components/ui/tag-input";
import { useToast } from "../../components/ui/toast";
import { getErrorMessage } from "../../lib/utils";

interface GroupForm {
  name: string;
  type: string;
  includeAll: boolean;
  excludeType: string;
  proxies: string[];
  icon: string;
}

const emptyForm: GroupForm = { name: "", type: "select", includeAll: true, excludeType: "", proxies: [], icon: "" };

function toForm(group: ProxyGroup): GroupForm {
  return {
    name: group.name,
    type: group.type || "select",
    includeAll: group["include-all"] === true,
    excludeType: String(group["exclude-type"] || ""),
    proxies: Array.isArray(group.proxies) ? group.proxies : [],
    icon: String(group.icon || ""),
  };
}

export function GroupsPage() {
  const configQuery = useConfigQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [sheetOpen, setSheetOpen] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [deletingIndex, setDeletingIndex] = useState<number | null>(null);
  const [form, setForm] = useState<GroupForm>(emptyForm);
  const [saving, setSaving] = useState(false);

  const groups = configQuery.data?.["proxy-groups"] || [];

  const openAdd = () => {
    setEditingIndex(null);
    setForm({ ...emptyForm, name: `新策略组 ${groups.length + 1}` });
    setSheetOpen(true);
  };

  const openEdit = (index: number) => {
    setEditingIndex(index);
    setForm(toForm(groups[index]));
    setSheetOpen(true);
  };

  const save = async () => {
    if (!configQuery.data) return;
    const name = form.name.trim();
    if (!name) {
      toast({ title: "策略组名称不能为空", tone: "error" });
      return;
    }
    const duplicate = groups.some((group, index) => index !== editingIndex && group.name === name);
    if (duplicate) {
      toast({ title: `策略组名称重复：${name}`, tone: "error" });
      return;
    }
    if (!form.includeAll && form.proxies.length === 0) {
      toast({ title: "未启用 include-all 时至少需要一个成员", tone: "error" });
      return;
    }

    setSaving(true);
    try {
      const next = [...groups];
      const original = editingIndex === null ? {} : groups[editingIndex];
      const merged: ProxyGroup = {
        ...original,
        name,
        type: form.type,
        "include-all": form.includeAll,
        proxies: form.proxies,
      };
      if (form.excludeType.trim()) merged["exclude-type"] = form.excludeType.trim();
      else delete merged["exclude-type"];
      if (form.icon.trim()) merged.icon = form.icon.trim();
      else delete merged.icon;

      if (editingIndex === null) next.push(merged);
      else next[editingIndex] = merged;

      await api.applyConfig({ ...configQuery.data, "proxy-groups": next });
      toast({ title: editingIndex === null ? "策略组已添加" : "策略组已更新", tone: "success" });
      setSheetOpen(false);
      await queryClient.invalidateQueries({ queryKey: queryKeys.config });
    } catch (error) {
      toast({ title: "保存策略组失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!configQuery.data || deletingIndex === null) return;
    const group = groups[deletingIndex];
    const referenced = configQuery.data.rules.some((rule) => rule.split(",").map((part) => part.trim()).includes(group.name));
    if (referenced) {
      toast({ title: "该策略组被规则引用，不能删除", description: "请先修改或删除相关规则。", tone: "error" });
      setDeletingIndex(null);
      return;
    }
    setSaving(true);
    try {
      const next = groups.filter((_, index) => index !== deletingIndex);
      await api.applyConfig({ ...configQuery.data, "proxy-groups": next });
      toast({ title: `已删除策略组：${group.name}`, tone: "success" });
      setDeletingIndex(null);
      await queryClient.invalidateQueries({ queryKey: queryKeys.config });
    } catch (error) {
      toast({ title: "删除策略组失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const deletingGroup = deletingIndex === null ? null : groups[deletingIndex];

  return (
    <div className="space-y-4">
      <PageTitle
        title="策略组"
        description="管理节点分组、类型与成员"
        action={<Button variant="primary" size="icon" aria-label="新增策略组" onClick={openAdd}><Plus size={18} /></Button>}
      />

      {configQuery.isLoading ? (
        <PageSkeleton />
      ) : configQuery.error ? (
        <ErrorState error={configQuery.error} onRetry={() => void configQuery.refetch()} />
      ) : groups.length === 0 ? (
        <EmptyState
          title="还没有策略组"
          description="创建策略组后，规则才能把流量导向不同节点集合。"
          icon={<Layers3 size={22} />}
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>新增策略组</Button>}
        />
      ) : (
        <div className="space-y-3">
          {groups.map((group, index) => (
            <Card key={`${group.name}-${index}`} className="overflow-hidden">
              <div className="p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h2 className="truncate text-[15px] font-semibold text-fg">{group.name}</h2>
                      <Badge tone="primary">{group.type}</Badge>
                      {group["include-all"] === true ? <Badge tone="neutral">include-all</Badge> : null}
                    </div>
                    <p className="mt-1 text-xs text-fg-muted">
                      {Array.isArray(group.proxies) && group.proxies.length ? group.proxies.join(" · ") : "未指定成员"}
                    </p>
                  </div>
                  <ActionsMenu
                    items={[
                      { label: "编辑", icon: <Pencil size={14} />, onSelect: () => openEdit(index) },
                      { label: "删除", icon: <Trash2 size={14} />, destructive: true, onSelect: () => setDeletingIndex(index) },
                    ]}
                  />
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <Sheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        title={editingIndex === null ? "新增策略组" : "编辑策略组"}
        description="保存后会立即写入 config.yaml 的 proxy-groups。"
        footer={
          <>
            <Button variant="secondary" onClick={() => setSheetOpen(false)} disabled={saving}>取消</Button>
            <Button variant="primary" onClick={() => void save()} loading={saving}>保存</Button>
          </>
        }
      >
        <div className="space-y-4">
          <Field label="名称" htmlFor="group-name" required>
            <Input id="group-name" value={form.name} onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, name: value })); }} />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="类型" htmlFor="group-type">
              <Select
                id="group-type"
                value={form.type}
                options={[
                  { value: "select", label: "select" },
                  { value: "url-test", label: "url-test" },
                  { value: "fallback", label: "fallback" },
                  { value: "load-balance", label: "load-balance" },
                  { value: "relay", label: "relay" },
                ]}
                onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, type: value })); }}
              />
            </Field>
            <Field label="exclude-type" htmlFor="group-exclude-type" hint="可选">
              <Input id="group-exclude-type" value={form.excludeType} placeholder="例如 dns" onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, excludeType: value })); }} />
            </Field>
          </div>
          <div className="flex items-center justify-between rounded-xl bg-surface-muted px-3 py-2.5">
            <div>
              <p className="text-sm font-medium text-fg">include-all</p>
              <p className="text-[11px] text-fg-muted">自动包含所有节点</p>
            </div>
            <Switch checked={form.includeAll} onCheckedChange={(checked) => setForm((current) => ({ ...current, includeAll: checked }))} />
          </div>
          <Field label="成员" htmlFor="group-proxies" hint="按顺序填写策略组或节点名称，点击上方 + 添加">
            <TagInput
              id="group-proxies"
              values={form.proxies}
              onChange={(proxies) => setForm((current) => ({ ...current, proxies }))}
              placeholder="例如：国外代理"
              mono
            />
          </Field>
          <Field label="图标 URL" htmlFor="group-icon" hint="可选，保留原图标">
            <Input id="group-icon" value={form.icon} placeholder="https://..." onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, icon: value })); }} />
          </Field>
        </div>
      </Sheet>

      <ConfirmDialog
        open={deletingGroup !== null}
        onOpenChange={(open) => { if (!open) setDeletingIndex(null); }}
        title={`删除策略组「${deletingGroup?.name || ""}」？`}
        description="如果该策略组正在被规则引用，后端会拒绝删除。保存成功后此操作不可撤销。"
        confirmLabel="删除"
        destructive
        loading={saving}
        onConfirm={() => void remove()}
      />
    </div>
  );
}
