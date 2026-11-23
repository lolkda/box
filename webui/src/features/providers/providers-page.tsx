import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Database, Pencil, Plus, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useConfigQuery } from "../../api/queries";
import type { RuleProvider } from "../../api/types";
import { ActionsMenu } from "../../components/ui/actions-menu";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Card } from "../../components/ui/card";
import { ConfirmDialog } from "../../components/ui/confirm-dialog";
import { EmptyState } from "../../components/ui/empty-state";
import { ErrorState } from "../../components/ui/error-state";
import { Field } from "../../components/ui/field";
import { Input, Textarea } from "../../components/ui/input";
import { PageTitle } from "../../components/ui/page-title";
import { Select } from "../../components/ui/select";
import { Sheet } from "../../components/ui/sheet";
import { PageSkeleton } from "../../components/ui/skeleton";
import { useToast } from "../../components/ui/toast";
import { ruleUsesProvider } from "../../lib/rules";
import { getErrorMessage } from "../../lib/utils";

interface ProviderForm {
  name: string;
  type: string;
  behavior: string;
  format: string;
  interval: string;
  url: string;
  path: string;
  proxy: string;
  payload: string;
}

const emptyForm: ProviderForm = {
  name: "",
  type: "http",
  behavior: "domain",
  format: "mrs",
  interval: "86400",
  url: "",
  path: "./rule_provider/new.mrs",
  proxy: "DIRECT",
  payload: "",
};

function toForm(name: string, provider: RuleProvider): ProviderForm {
  return {
    name,
    type: provider.type || "http",
    behavior: provider.behavior || "domain",
    format: provider.format || "yaml",
    interval: String(provider.interval ?? 86400),
    url: provider.url || "",
    path: provider.path || "",
    proxy: provider.proxy || "DIRECT",
    payload: Array.isArray(provider.payload) ? provider.payload.join("\n") : "",
  };
}

export function ProvidersPage() {
  const configQuery = useConfigQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [sheetOpen, setSheetOpen] = useState(false);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [saving, setSaving] = useState(false);
  const [deletingName, setDeletingName] = useState<string | null>(null);

  const config = configQuery.data;
  const providers = config?.["rule-providers"] || {};
  const entries = Object.entries(providers);

  const openAdd = () => {
    setEditingName(null);
    setForm({ ...emptyForm, name: `规则集${entries.length + 1}` });
    setSheetOpen(true);
  };

  const openEdit = (name: string, provider: RuleProvider) => {
    setEditingName(name);
    setForm(toForm(name, provider));
    setSheetOpen(true);
  };

  const save = async () => {
    if (!config) return;
    const name = form.name.trim();
    if (!name) {
      toast({ title: "规则集名称不能为空", tone: "error" });
      return;
    }
    if (editingName === null && name in providers) {
      toast({ title: `规则集已存在：${name}`, tone: "error" });
      return;
    }
    if (editingName !== null && name !== editingName && name in providers) {
      toast({ title: `规则集已存在：${name}`, tone: "error" });
      return;
    }
    if (!/^\d+$/.test(form.interval.trim())) {
      toast({ title: "interval 必须是非负整数", tone: "error" });
      return;
    }
    if (form.type === "http" && !/^https?:\/\//i.test(form.url.trim())) {
      toast({ title: "HTTP 规则集需要合法 URL", tone: "error" });
      return;
    }
    if (form.type !== "inline" && !form.path.trim()) {
      toast({ title: "请填写规则集 path", tone: "error" });
      return;
    }

    setSaving(true);
    try {
      const original = editingName ? providers[editingName] : {};
      const nextProvider: RuleProvider = {
        ...original,
        type: form.type,
        behavior: form.behavior,
        format: form.format,
        interval: Number(form.interval) || 0,
      };
      if (form.type === "http") nextProvider.url = form.url.trim();
      else delete nextProvider.url;
      if (form.path.trim()) nextProvider.path = form.path.trim();
      else delete nextProvider.path;
      if (form.proxy.trim()) nextProvider.proxy = form.proxy.trim();
      else delete nextProvider.proxy;
      if (form.type === "inline") {
        nextProvider.payload = form.payload.split("\n").map((line) => line.trim()).filter(Boolean);
      } else {
        delete nextProvider.payload;
      }

      const next: Record<string, RuleProvider> = {};
      for (const [key, value] of entries) {
        if (key === editingName) {
          next[name] = nextProvider;
        } else {
          next[key] = value;
        }
      }
      if (editingName === null) next[name] = nextProvider;

      await api.applyConfig({ ...config, "rule-providers": next });
      toast({ title: editingName === null ? "规则集已添加" : "规则集已更新", tone: "success" });
      setSheetOpen(false);
      await queryClient.invalidateQueries({ queryKey: queryKeys.config });
    } catch (error) {
      toast({ title: "保存规则集失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!config || !deletingName) return;
    const referenced = config.rules.some((rule) => ruleUsesProvider(rule, deletingName));
    if (referenced) {
      toast({ title: "该规则集被规则引用，不能删除", description: "请先修改或删除相关 RULE-SET 规则。", tone: "error" });
      setDeletingName(null);
      return;
    }
    setSaving(true);
    try {
      const next = { ...providers };
      delete next[deletingName];
      await api.applyConfig({ ...config, "rule-providers": next });
      toast({ title: `已删除规则集：${deletingName}`, tone: "success" });
      setDeletingName(null);
      await queryClient.invalidateQueries({ queryKey: queryKeys.config });
    } catch (error) {
      toast({ title: "删除规则集失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-4">
      <PageTitle
        title="规则集"
        description="管理规则集来源、格式与更新间隔"
        action={<Button variant="primary" size="icon" aria-label="新增规则集" onClick={openAdd}><Plus size={18} /></Button>}
      />

      {configQuery.isLoading ? (
        <PageSkeleton />
      ) : configQuery.error ? (
        <ErrorState error={configQuery.error} onRetry={() => void configQuery.refetch()} />
      ) : entries.length === 0 ? (
        <EmptyState
          title="还没有规则集"
          description="规则集可以让规则复用外部维护的域名或 IP 列表。"
          icon={<Database size={22} />}
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>新增规则集</Button>}
        />
      ) : (
        <div className="space-y-3">
          {entries.map(([name, provider]) => (
            <Card key={name} className="overflow-hidden">
              <div className="p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <h2 className="truncate text-[15px] font-semibold text-fg">{name}</h2>
                      <Badge tone="primary">{provider.type}</Badge>
                      <Badge tone="neutral">{provider.behavior}</Badge>
                      <Badge tone="info">{provider.format}</Badge>
                    </div>
                    <p className="mt-1 truncate font-mono text-[11px] text-fg-muted">{provider.url || provider.path || "inline payload"}</p>
                    <p className="mt-1 text-[11px] text-fg-subtle">每 {provider.interval || 0} 秒更新</p>
                  </div>
                  <ActionsMenu
                    items={[
                      { label: "编辑", icon: <Pencil size={14} />, onSelect: () => openEdit(name, provider) },
                      { label: "删除", icon: <Trash2 size={14} />, destructive: true, onSelect: () => setDeletingName(name) },
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
        title={editingName === null ? "新增规则集" : "编辑规则集"}
        description="保存后写入 config.yaml 的 rule-providers。"
        footer={
          <>
            <Button variant="secondary" onClick={() => setSheetOpen(false)} disabled={saving}>取消</Button>
            <Button variant="primary" onClick={() => void save()} loading={saving}>保存</Button>
          </>
        }
      >
        <div className="space-y-4">
          <Field label="名称" htmlFor="provider-name" required>
            <Input id="provider-name" value={form.name} onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, name: value })); }} />
          </Field>
          <div className="grid grid-cols-3 gap-2">
            <Field label="type" htmlFor="provider-type">
              <Select id="provider-type" value={form.type} options={[
                { value: "http", label: "http" },
                { value: "file", label: "file" },
                { value: "inline", label: "inline" },
              ]} onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, type: value })); }} />
            </Field>
            <Field label="behavior" htmlFor="provider-behavior">
              <Select id="provider-behavior" value={form.behavior} options={[
                { value: "domain", label: "domain" },
                { value: "ipcidr", label: "ipcidr" },
                { value: "classical", label: "classical" },
              ]} onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, behavior: value })); }} />
            </Field>
            <Field label="format" htmlFor="provider-format">
              <Select id="provider-format" value={form.format} options={[
                { value: "mrs", label: "mrs" },
                { value: "yaml", label: "yaml" },
                { value: "text", label: "text" },
              ]} onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, format: value })); }} />
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <Field label="更新间隔（秒）" htmlFor="provider-interval" required>
              <Input id="provider-interval" value={form.interval} inputMode="numeric" onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, interval: value })); }} />
            </Field>
            <Field label="proxy" htmlFor="provider-proxy" hint="可选">
              <Input id="provider-proxy" value={form.proxy} placeholder="DIRECT" onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, proxy: value })); }} />
            </Field>
          </div>
          {form.type === "http" ? (
            <Field label="URL" htmlFor="provider-url" required>
              <Input id="provider-url" value={form.url} placeholder="https://..." inputMode="url" className="font-mono text-xs" onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, url: value })); }} />
            </Field>
          ) : null}
          {form.type !== "inline" ? (
            <Field label="path" htmlFor="provider-path" required hint="相对于 mihomo 配置目录，例如 ./rule_provider/cn.mrs">
              <Input id="provider-path" value={form.path} placeholder="./rule_provider/..." className="font-mono text-xs" onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, path: value })); }} />
            </Field>
          ) : (
            <Field label="payload" htmlFor="provider-payload" hint="每行一条规则">
              <Textarea id="provider-payload" value={form.payload} className="font-mono text-xs" onChange={(event) => { const value = event.currentTarget.value; setForm((current) => ({ ...current, payload: value })); }} />
            </Field>
          )}
        </div>
      </Sheet>

      <ConfirmDialog
        open={deletingName !== null}
        onOpenChange={(open) => { if (!open) setDeletingName(null); }}
        title={`删除规则集「${deletingName || ""}」？`}
        description="如果规则正在引用该规则集，后端会拒绝删除。此操作不可撤销。"
        confirmLabel="删除"
        destructive
        loading={saving}
        onConfirm={() => void remove()}
      />
    </div>
  );
}
