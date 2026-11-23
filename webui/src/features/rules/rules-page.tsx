import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, ListTree, Pencil, Plus, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useConfigQuery } from "../../api/queries";
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
import { TagInput } from "../../components/ui/tag-input";
import { useToast } from "../../components/ui/toast";
import {
  createRuleDraft,
  parseRule,
  ruleTypes,
  serializeRule,
  validateRuleDraft,
  type RuleDraft,
} from "../../lib/rules";
import { getErrorMessage } from "../../lib/utils";

function RuleRow({
  rule,
  index,
  total,
  onEdit,
  onDelete,
  onMove,
}: {
  rule: string;
  index: number;
  total: number;
  onEdit: () => void;
  onDelete: () => void;
  onMove: (direction: -1 | 1) => void;
}) {
  const draft = parseRule(rule);
  return (
    <Card className="overflow-hidden">
      <div className="flex items-start gap-3 p-4">
        <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-surface-muted text-[11px] font-semibold text-fg-muted">
          {index + 1}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <Badge tone="primary">{draft.type || "RAW"}</Badge>
            {draft.mode === "structured" && draft.target ? <Badge tone="neutral">→ {draft.target}</Badge> : null}
            {draft.options.map((option) => (
              <Badge tone="info" key={option}>{option}</Badge>
            ))}
          </div>
          <p className="mt-2 break-all font-mono text-[11px] leading-5 text-fg-muted">{rule}</p>
        </div>
        <ActionsMenu
          items={[
            { label: "编辑", icon: <Pencil size={14} />, onSelect: onEdit },
            { label: "上移", icon: <ArrowUp size={14} />, onSelect: () => onMove(-1), disabled: index === 0 },
            { label: "下移", icon: <ArrowDown size={14} />, onSelect: () => onMove(1), disabled: index === total - 1 },
            { label: "删除", icon: <Trash2 size={14} />, destructive: true, onSelect: onDelete },
          ]}
        />
      </div>
    </Card>
  );
}

export function RulesPage() {
  const configQuery = useConfigQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [sheetOpen, setSheetOpen] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [draft, setDraft] = useState<RuleDraft>(createRuleDraft());
  const [saving, setSaving] = useState(false);
  const [deletingIndex, setDeletingIndex] = useState<number | null>(null);

  const config = configQuery.data;
  const rules = config?.rules || [];
  const groups = config?.["proxy-groups"] || [];
  const providers = config?.["rule-providers"] || {};
  const providerNames = Object.keys(providers);
  const groupTargets = useMemo(
    () => [
      ...groups.map((group) => ({ value: group.name, label: group.name })),
      { value: "DIRECT", label: "DIRECT" },
      { value: "REJECT", label: "REJECT" },
      { value: "REJECT-DROP", label: "REJECT-DROP" },
      { value: "PASS", label: "PASS" },
      { value: "COMPATIBLE", label: "COMPATIBLE" },
    ],
    [groups],
  );

  const openAdd = () => {
    setEditingIndex(null);
    setDraft(createRuleDraft());
    setSheetOpen(true);
  };

  const openEdit = (index: number) => {
    setEditingIndex(index);
    setDraft(parseRule(rules[index]));
    setSheetOpen(true);
  };

  const persist = async (nextRules: string[], successMessage: string) => {
    if (!config) return;
    setSaving(true);
    try {
      await api.applyConfig({ ...config, rules: nextRules });
      toast({ title: successMessage, tone: "success" });
      setSheetOpen(false);
      await queryClient.invalidateQueries({ queryKey: queryKeys.config });
    } catch (error) {
      toast({ title: "保存规则失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setSaving(false);
    }
  };

  const save = async () => {
    const validation = validateRuleDraft(draft, groups, providers);
    if (validation) {
      toast({ title: validation, tone: "error" });
      return;
    }
    const serialized = serializeRule(draft);
    const next = [...rules];
    if (editingIndex === null) next.push(serialized);
    else next[editingIndex] = serialized;
    await persist(next, editingIndex === null ? "规则已添加" : "规则已更新");
  };

  const remove = async () => {
    if (deletingIndex === null) return;
    const next = rules.filter((_, index) => index !== deletingIndex);
    const rule = rules[deletingIndex];
    setDeletingIndex(null);
    await persist(next, `已删除规则：${rule.slice(0, 48)}`);
  };

  const move = async (index: number, direction: -1 | 1) => {
    const target = index + direction;
    if (target < 0 || target >= rules.length) return;
    const next = [...rules];
    [next[index], next[target]] = [next[target], next[index]];
    await persist(next, "规则顺序已更新");
  };

  const preview = serializeRule(draft);
  const editingRule = editingIndex === null ? "" : rules[editingIndex];

  return (
    <div className="space-y-4">
      <PageTitle
        title="规则"
        description="从上到下匹配；结构化编辑并保留高级原文模式"
        action={<Button variant="primary" size="icon" aria-label="新增规则" onClick={openAdd}><Plus size={18} /></Button>}
      />

      {configQuery.isLoading ? (
        <PageSkeleton />
      ) : configQuery.error ? (
        <ErrorState error={configQuery.error} onRetry={() => void configQuery.refetch()} />
      ) : rules.length === 0 ? (
        <EmptyState
          title="还没有规则"
          description="规则决定流量走哪个策略组。至少保留一条 MATCH 兜底规则。"
          icon={<ListTree size={22} />}
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>新增规则</Button>}
        />
      ) : (
        <div className="space-y-3">
          {rules.map((rule, index) => (
            <RuleRow
              key={`${rule}-${index}`}
              rule={rule}
              index={index}
              total={rules.length}
              onEdit={() => openEdit(index)}
              onDelete={() => setDeletingIndex(index)}
              onMove={(direction) => void move(index, direction)}
            />
          ))}
        </div>
      )}

      <Sheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        title={editingIndex === null ? "新增规则" : "编辑规则"}
        description="结构化模式会即时生成规则字符串，保存前请检查预览。"
        footer={
          <>
            <Button variant="secondary" onClick={() => setSheetOpen(false)} disabled={saving}>取消</Button>
            <Button variant="primary" onClick={() => void save()} loading={saving}>保存</Button>
          </>
        }
      >
        <div className="space-y-4">
          <div className="grid grid-cols-2 rounded-xl bg-surface-muted p-1">
            <button
              type="button"
              onClick={() => setDraft((current) => ({ ...current, mode: "structured" }))}
              className={`h-9 rounded-lg text-xs font-medium transition-colors ${draft.mode === "structured" ? "bg-surface text-fg shadow-sm" : "text-fg-muted"}`}
            >
              结构化
            </button>
            <button
              type="button"
              onClick={() => setDraft((current) => ({ ...current, mode: "raw" }))}
              className={`h-9 rounded-lg text-xs font-medium transition-colors ${draft.mode === "raw" ? "bg-surface text-fg shadow-sm" : "text-fg-muted"}`}
            >
              高级原文
            </button>
          </div>

          {draft.mode === "raw" ? (
            <Field label="规则原文" htmlFor="rule-raw" required hint="支持任意 mihomo 规则语法；引用错误可能在后端被拒绝。">
              <Textarea
                id="rule-raw"
                value={draft.raw}
                placeholder="DOMAIN-SUFFIX,example.com,国外代理"
                className="font-mono text-xs"
                onChange={(event) => { const value = event.currentTarget.value; setDraft((current) => ({ ...current, raw: value })); }}
              />
            </Field>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-3">
                <Field label="类型" htmlFor="rule-type">
                  <Select
                    id="rule-type"
                    value={draft.type}
                    options={ruleTypes.map((type) => ({ value: type.value, label: type.label }))}
                    onChange={(event) => { const value = event.currentTarget.value; setDraft((current) => ({ ...current, type: value, value: "" })); }}
                  />
                </Field>
                <Field label="目标策略组" htmlFor="rule-target" required>
                  <Select
                    id="rule-target"
                    value={draft.target}
                    options={[{ value: "", label: "请选择" }, ...groupTargets]}
                    onChange={(event) => { const value = event.currentTarget.value; setDraft((current) => ({ ...current, target: value })); }}
                  />
                </Field>
              </div>
              {draft.type !== "MATCH" ? (
                <Field
                  label={ruleTypes.find((type) => type.value === draft.type)?.valueLabel || "内容"}
                  htmlFor="rule-value"
                  required
                >
                  {draft.type === "RULE-SET" ? (
                    <Select
                      id="rule-value"
                      value={draft.value}
                      options={[{ value: "", label: "请选择规则集" }, ...providerNames.map((name) => ({ value: name, label: name }))]}
                      onChange={(event) => { const value = event.currentTarget.value; setDraft((current) => ({ ...current, value })); }}
                    />
                  ) : (
                    <Input
                      id="rule-value"
                      value={draft.value}
                      placeholder={ruleTypes.find((type) => type.value === draft.type)?.valuePlaceholder}
                      className="font-mono text-xs"
                      onChange={(event) => { const value = event.currentTarget.value; setDraft((current) => ({ ...current, value })); }}
                    />
                  )}
                </Field>
              ) : null}
              <Field label="附加选项" htmlFor="rule-options" hint="例如 no-resolve、src；可选">
                <TagInput
                  id="rule-options"
                  values={draft.options}
                  onChange={(options) => setDraft((current) => ({ ...current, options }))}
                  placeholder="输入选项后按回车"
                  mono
                />
              </Field>
            </>
          )}

          <div className="rounded-xl border border-border bg-surface-muted p-3">
            <p className="mb-1 text-[10px] font-medium uppercase tracking-wider text-fg-subtle">预览</p>
            <code className="block break-all font-mono text-[11px] leading-5 text-fg">{preview || "—"}</code>
          </div>
          {editingRule ? <p className="text-[11px] text-fg-subtle">原规则：{editingRule}</p> : null}
        </div>
      </Sheet>

      <ConfirmDialog
        open={deletingIndex !== null}
        onOpenChange={(open) => { if (!open) setDeletingIndex(null); }}
        title="删除这条规则？"
        description="删除后会立即写入配置文件，且不可撤销。"
        confirmLabel="删除"
        destructive
        loading={saving}
        onConfirm={() => void remove()}
      />
    </div>
  );
}
