import { useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  Pencil,
  Plus,
  RotateCcw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { api } from "../../api/client";
import { queryKeys, useNetworkRulesQuery } from "../../api/queries";
import type {
  NetworkRule,
  NetworkRuleAction,
  NetworkRuleDstType,
  NetworkRuleFamily,
  NetworkRuleMatch,
  NetworkRuleMode,
  NetworkRuleProtocol,
  NetworkRuleScope,
  NetworkRulesPreview,
  NetworkSystemRule,
} from "../../api/types";
import { StickyActionBar } from "../../components/layout/sticky-action-bar";
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

const DRAFT_KEY = "box.network-rules.draft.v1";
const ALL_MODES: NetworkRuleMode[] = ["tun", "tproxy", "redirect", "mixed", "enhance"];
const MODE_LABELS: Record<NetworkRuleMode, string> = {
  tun: "TUN",
  tproxy: "TProxy",
  redirect: "Redirect",
  mixed: "Mixed",
  enhance: "Enhance",
};
const SCOPE_LABELS: Record<NetworkRuleScope, string> = {
  local: "本机",
  forward: "转发/TUN",
  dns: "DNS",
  filter: "过滤",
};
const ACTION_LABELS: Record<NetworkRuleAction, string> = { bypass: "旁路", reject: "拒绝" };
const FAMILY_LABELS: Record<NetworkRuleFamily, string> = { ipv4: "IPv4", ipv6: "IPv6", both: "双栈" };
const DST_TYPE_LABELS: Record<NetworkRuleDstType, string> = {
  local: "本机",
  multicast: "组播",
  broadcast: "广播",
};

interface DraftState {
  baseRevision: number;
  rules: NetworkRule[];
  tombstones: string[];
  savedAt: number;
}

function emptyMatch(): NetworkRuleMatch {
  return {
    dst_cidrs: [],
    src_cidrs: [],
    dst_ports: [],
    src_ports: [],
    src_ifaces: [],
    dst_ifaces: [],
    uids: [],
    gids: [],
    dst_types: [],
  };
}

function emptyRule(): NetworkRule {
  return {
    id: `user-${Date.now()}-${Math.floor(Math.random() * 1000)}`,
    name: "新规则",
    enabled: true,
    action: "bypass",
    scopes: ["local", "forward"],
    modes: [...ALL_MODES],
    family: "both",
    protocol: "any",
    match: emptyMatch(),
    origin: "user",
  };
}

function hasMatch(match: NetworkRuleMatch): boolean {
  return Object.values(match).some((value) => Array.isArray(value) && value.length > 0);
}

function validateRule(rule: NetworkRule): string | null {
  if (!rule.name.trim()) return "规则名称不能为空";
  if (rule.scopes.length === 0) return "至少选择一个作用层";
  if (rule.modes.length === 0) return "至少选择一个网络模式";
  if (!hasMatch(rule.match)) return "至少填写一个匹配条件";
  if (rule.action === "reject" && rule.scopes.includes("dns")) return "DNS 作用层不能使用拒绝动作";
  if (rule.action === "bypass" && rule.scopes.includes("filter")) return "过滤作用层不能使用旁路动作";
  return null;
}

function ruleSummary(rule: NetworkRule): string {
  const parts: string[] = [];
  const m = rule.match;
  if (m.dst_cidrs.length) parts.push(`dst ${m.dst_cidrs.join(", ")}`);
  if (m.src_cidrs.length) parts.push(`src ${m.src_cidrs.join(", ")}`);
  if (m.dst_ports.length) parts.push(`dport ${m.dst_ports.join(", ")}`);
  if (m.src_ports.length) parts.push(`sport ${m.src_ports.join(", ")}`);
  if (m.uids.length) parts.push(`uid ${m.uids.join(", ")}`);
  if (m.gids.length) parts.push(`gid ${m.gids.join(", ")}`);
  if (m.src_ifaces.length) parts.push(`in ${m.src_ifaces.join(", ")}`);
  if (m.dst_ifaces.length) parts.push(`out ${m.dst_ifaces.join(", ")}`);
  if (m.dst_types.length) parts.push(`type ${m.dst_types.join(", ")}`);
  return parts.join(" · ") || "未填写匹配";
}

function withMatch(rule: NetworkRule, match: NetworkRuleMatch): NetworkRule {
  return { ...rule, match };
}

function RuleRow({
  rule,
  index,
  total,
  onEdit,
  onToggle,
  onDelete,
  onMove,
}: {
  rule: NetworkRule;
  index: number;
  total: number;
  onEdit: () => void;
  onToggle: (enabled: boolean) => void;
  onDelete: () => void;
  onMove: (direction: -1 | 1) => void;
}) {
  return (
    <Card className="overflow-hidden">
      <div className="flex items-start gap-3 p-4">
        <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-surface-muted text-[11px] font-semibold text-fg-muted">
          {index + 1}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className={`truncate text-sm font-medium ${rule.enabled ? "text-fg" : "text-fg-subtle line-through"}`}>
              {rule.name}
            </span>
            <Badge tone={rule.action === "reject" ? "danger" : "info"}>{ACTION_LABELS[rule.action]}</Badge>
            {rule.origin === "default" ? <Badge tone="neutral">默认</Badge> : null}
          </div>
          <p className="mt-1 break-all text-[11px] leading-5 text-fg-muted">{ruleSummary(rule)}</p>
          <div className="mt-2 flex flex-wrap gap-1">
            {rule.scopes.map((scope) => <Badge key={scope} tone="neutral">{SCOPE_LABELS[scope]}</Badge>)}
            {rule.modes.length === ALL_MODES.length
              ? <Badge tone="neutral">全部模式</Badge>
              : rule.modes.map((mode) => <Badge key={mode} tone="info">{MODE_LABELS[mode]}</Badge>)}
            <Badge tone="neutral">{FAMILY_LABELS[rule.family]}</Badge>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <Switch checked={rule.enabled} onCheckedChange={onToggle} aria-label={`启用 ${rule.name}`} />
          <ActionsMenu
            items={[
              { label: "编辑", icon: <Pencil size={14} />, onSelect: onEdit },
              { label: "上移", icon: <ArrowUp size={14} />, onSelect: () => onMove(-1), disabled: index === 0 },
              { label: "下移", icon: <ArrowDown size={14} />, onSelect: () => onMove(1), disabled: index === total - 1 },
              { label: "删除", icon: <Trash2 size={14} />, destructive: true, onSelect: onDelete },
            ]}
          />
        </div>
      </div>
    </Card>
  );
}

function SystemRow({ rule }: { rule: NetworkSystemRule }) {
  return (
    <div className="flex items-start gap-3 border-b border-border/70 px-4 py-3 last:border-b-0">
      <ShieldCheck size={16} className="mt-0.5 shrink-0 text-primary" />
      <div className="min-w-0 flex-1">
        <p className="text-xs font-medium text-fg">{rule.name}</p>
        <p className="mt-0.5 text-[11px] leading-4 text-fg-muted">{rule.description}</p>
      </div>
      <Badge tone="neutral">只读</Badge>
    </div>
  );
}

function ChoiceChip({
  active,
  children,
  onClick,
}: {
  active: boolean;
  children: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex items-center gap-1 rounded-lg border px-2.5 py-1.5 text-[11px] transition-colors ${
        active
          ? "border-primary/40 bg-primary-soft text-primary"
          : "border-border bg-surface-muted text-fg-muted hover:text-fg"
      }`}
    >
      {active ? <Check size={12} /> : null}
      {children}
    </button>
  );
}

export function NetworkRulesPage() {
  const query = useNetworkRulesQuery();
  const queryClient = useQueryClient();
  const toast = useToast();
  const initialized = useRef(false);
  const [rules, setRules] = useState<NetworkRule[]>([]);
  const [tombstones, setTombstones] = useState<string[]>([]);
  const [baseRevision, setBaseRevision] = useState(0);
  const [serverRevision, setServerRevision] = useState(0);
  const [draftConflict, setDraftConflict] = useState(false);
  const [formRule, setFormRule] = useState<NetworkRule | null>(null);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [deletingIndex, setDeletingIndex] = useState<number | null>(null);
  const [preview, setPreview] = useState<NetworkRulesPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [applying, setApplying] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const data = query.data;
  const dirty = useMemo(() => {
    if (!data) return false;
    return (
      baseRevision !== data.revision ||
      JSON.stringify(rules) !== JSON.stringify(data.rules) ||
      JSON.stringify(tombstones) !== JSON.stringify(data.tombstones)
    );
  }, [baseRevision, data, rules, tombstones]);

  useEffect(() => {
    if (!data || initialized.current) return;
    setServerRevision(data.revision);
    const raw = typeof window === "undefined" ? null : window.localStorage.getItem(DRAFT_KEY);
    if (raw) {
      try {
        const saved = JSON.parse(raw) as DraftState;
        if (saved.baseRevision === data.revision && Array.isArray(saved.rules) && Array.isArray(saved.tombstones)) {
          setRules(saved.rules);
          setTombstones(saved.tombstones);
          setBaseRevision(saved.baseRevision);
          initialized.current = true;
          return;
        }
        setDraftConflict(true);
      } catch {
        // 损坏的草稿回退到服务端数据。
      }
    }
    setRules(data.rules);
    setTombstones(data.tombstones);
    setBaseRevision(data.revision);
    initialized.current = true;
  }, [data]);

  useEffect(() => {
    if (!initialized.current) return;
    if (!dirty) {
      if (typeof window !== "undefined") window.localStorage.removeItem(DRAFT_KEY);
      return;
    }
    const draft: DraftState = { baseRevision, rules, tombstones, savedAt: Date.now() };
    if (typeof window !== "undefined") window.localStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
  }, [baseRevision, dirty, rules, tombstones]);

  const updateRules = (next: NetworkRule[]) => {
    setRules(next);
    setPreview(null);
  };

  const openAdd = () => {
    setEditingIndex(null);
    setFormRule(emptyRule());
    setAdvancedOpen(false);
  };

  const openEdit = (index: number) => {
    setEditingIndex(index);
    setFormRule({ ...rules[index], match: { ...rules[index].match } });
    setAdvancedOpen(false);
  };

  const saveForm = () => {
    if (!formRule) return;
    const error = validateRule(formRule);
    if (error) {
      toast({ title: "规则不完整", description: error, tone: "error" });
      return;
    }
    const next = [...rules];
    if (editingIndex === null) next.push(formRule);
    else next[editingIndex] = formRule;
    updateRules(next);
    setFormRule(null);
  };

  const toggle = (index: number, enabled: boolean) => {
    updateRules(rules.map((rule, itemIndex) => (itemIndex === index ? { ...rule, enabled } : rule)));
  };

  const remove = () => {
    if (deletingIndex === null) return;
    const rule = rules[deletingIndex];
    updateRules(rules.filter((_, index) => index !== deletingIndex));
    if (rule.origin === "default") {
      setTombstones((current) => [...new Set([...current, rule.id])].sort());
    }
    setDeletingIndex(null);
  };

  const move = (index: number, direction: -1 | 1) => {
    const target = index + direction;
    if (target < 0 || target >= rules.length) return;
    const next = [...rules];
    [next[index], next[target]] = [next[target], next[index]];
    updateRules(next);
  };

  const discardDraft = () => {
    if (!data) return;
    setRules(data.rules);
    setTombstones(data.tombstones);
    setBaseRevision(data.revision);
    setServerRevision(data.revision);
    setPreview(null);
    setDraftConflict(false);
    if (typeof window !== "undefined") window.localStorage.removeItem(DRAFT_KEY);
  };

  const openPreview = async () => {
    setPreviewing(true);
    try {
      const result = await api.previewNetworkRules({
        base_revision: baseRevision,
        rules,
        tombstones,
      });
      setPreview(result);
    } catch (error) {
      toast({ title: "预览失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setPreviewing(false);
    }
  };

  const apply = async () => {
    setApplying(true);
    try {
      const result = await api.applyNetworkRules({
        base_revision: baseRevision,
        rules,
        tombstones,
      });
      setRules(result.rules);
      setTombstones(result.tombstones);
      setBaseRevision(result.revision);
      setServerRevision(result.revision);
      setPreview(null);
      initialized.current = true;
      if (typeof window !== "undefined") window.localStorage.removeItem(DRAFT_KEY);
      setDraftConflict(false);
      toast({
        title: result.applied === false ? "规则已保存" : "网络规则已应用",
        description: result.applied === false ? "服务未运行，下次启动时生效" : "规则已写入当前网络计划",
        tone: "success",
      });
      await queryClient.invalidateQueries({ queryKey: queryKeys.networkRules });
    } catch (error) {
      toast({ title: "应用失败，已保留草稿", description: getErrorMessage(error), tone: "error" });
    } finally {
      setApplying(false);
    }
  };

  const reset = async () => {
    setResetting(true);
    try {
      const result = await api.resetNetworkRules();
      setRules(result.rules);
      setTombstones(result.tombstones);
      setBaseRevision(result.revision);
      setServerRevision(result.revision);
      setPreview(null);
      if (typeof window !== "undefined") window.localStorage.removeItem(DRAFT_KEY);
      setDraftConflict(false);
      toast({ title: "已恢复默认规则", tone: "success" });
      await queryClient.invalidateQueries({ queryKey: queryKeys.networkRules });
    } catch (error) {
      toast({ title: "恢复默认失败", description: getErrorMessage(error), tone: "error" });
    } finally {
      setResetting(false);
    }
  };

  if (query.isLoading) return <PageSkeleton />;
  if (query.error) return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  if (!data) return null;

  const systemRules = data.system || [];
  const conflict = draftConflict || serverRevision !== data.revision;

  return (
    <div className="space-y-4 pb-8">
      <PageTitle
        title="网络规则"
        description="本机、转发/TUN、DNS 与过滤规则；系统保护只读"
        action={<Button variant="primary" size="icon" aria-label="新增网络规则" onClick={openAdd}><Plus size={18} /></Button>}
      />

      <Card>
        <div className="border-b border-border px-4 py-3">
          <p className="flex items-center gap-2 text-xs font-medium text-fg"><ShieldCheck size={15} className="text-primary" /> 系统保护</p>
          <p className="mt-1 text-[11px] leading-4 text-fg-muted">固定先于用户规则执行，不能编辑、删除或排序。</p>
        </div>
        <div>{systemRules.map((rule) => <SystemRow key={rule.id} rule={rule} />)}</div>
      </Card>

      {conflict ? (
        <Card>
          <div className="flex items-center justify-between gap-3 p-4">
            <div>
              <p className="text-xs font-medium text-danger">服务端规则已更新</p>
              <p className="mt-1 text-[11px] text-fg-muted">当前草稿基于旧 revision，应用前请重新加载或丢弃。</p>
            </div>
            <Button size="sm" variant="secondary" onClick={discardDraft}>加载服务端</Button>
          </div>
        </Card>
      ) : null}

      {dirty ? (
        <Card>
          <div className="flex items-center justify-between gap-3 p-4">
            <div>
              <p className="text-xs font-medium text-fg">有未应用修改</p>
              <p className="mt-1 text-[11px] text-fg-muted">修改只保存在本机草稿，预览后可原子应用。</p>
            </div>
            <Button size="sm" variant="ghost" onClick={discardDraft}>丢弃</Button>
          </div>
        </Card>
      ) : null}

      {rules.length === 0 ? (
        <EmptyState
          title="还没有用户规则"
          description="系统保护仍然生效。可以新增一条旁路或拒绝规则。"
          icon={<ShieldCheck size={22} />}
          action={<Button variant="primary" onClick={openAdd} icon={<Plus size={16} />}>新增规则</Button>}
        />
      ) : (
        <div className="space-y-3">
          {rules.map((rule, index) => (
            <RuleRow
              key={rule.id}
              rule={rule}
              index={index}
              total={rules.length}
              onEdit={() => openEdit(index)}
              onToggle={(enabled) => toggle(index, enabled)}
              onDelete={() => setDeletingIndex(index)}
              onMove={(direction) => move(index, direction)}
            />
          ))}
        </div>
      )}

      <StickyActionBar label="网络规则操作栏">
        <Button
          variant="secondary"
          size="md"
          className="h-10 px-3 text-xs"
          onClick={() => void reset()}
          loading={resetting}
        >
          <RotateCcw size={15} /> 恢复默认
        </Button>
        <Button
          variant="primary"
          size="md"
          className="h-10 px-3 text-xs"
          onClick={() => void openPreview()}
          loading={previewing}
          disabled={!dirty || conflict}
        >
          预览变更
        </Button>
      </StickyActionBar>

      {preview ? (
        <Card>
          <div className="border-b border-border px-4 py-3">
            <p className="text-sm font-medium text-fg">变更预览</p>
            <p className="mt-1 text-[11px] text-fg-muted">
              新增 {preview.diff.added} · 修改 {preview.diff.updated} · 删除 {preview.diff.removed}
              {preview.diff.reordered ? " · 顺序变化" : ""}
            </p>
          </div>
          <div className="space-y-2 px-4 py-3 text-[11px] leading-5 text-fg-muted">
            {preview.warnings.length ? preview.warnings.map((warning) => (
              <p key={warning} className="rounded-lg bg-warning-soft px-2.5 py-2 text-warning">{warning}</p>
            )) : <p>校验通过，可以应用。</p>}
            {preview.plan?.available ? (
              <p>
                当前模式：{preview.plan.effective_mode} · 计划 {preview.plan.rule_count} 条
                {preview.plan.warnings?.length ? ` · ${preview.plan.warnings.join("；")}` : ""}
              </p>
            ) : null}
            {preview.diff.added_ids.length ? <p>新增 ID：{preview.diff.added_ids.join(", ")}</p> : null}
            {preview.diff.removed_ids.length ? <p>删除 ID：{preview.diff.removed_ids.join(", ")}</p> : null}
          </div>
          <div className="flex justify-end gap-2 border-t border-border px-4 py-3">
            <Button variant="secondary" onClick={() => setPreview(null)}>取消</Button>
            <Button variant="primary" onClick={() => void apply()} loading={applying}>应用规则</Button>
          </div>
        </Card>
      ) : null}

      <Sheet
        open={formRule !== null}
        onOpenChange={(open) => { if (!open) setFormRule(null); }}
        title={editingIndex === null ? "新增网络规则" : "编辑网络规则"}
        description="规则是一条语义化匹配；系统保护不可在这里修改。"
        footer={
          <>
            <Button variant="secondary" onClick={() => setFormRule(null)}>取消</Button>
            <Button variant="primary" onClick={saveForm}>保存到草稿</Button>
          </>
        }
      >
        {formRule ? (
          <div className="space-y-4">
            <Field label="名称" htmlFor="rule-name">
              <Input
                id="rule-name"
                value={formRule.name}
                onChange={(event) => setFormRule({ ...formRule, name: event.currentTarget.value })}
                placeholder="例如：局域网旁路"
              />
            </Field>

            <div className="grid grid-cols-2 gap-3">
              <Field label="动作">
                <Select
                  value={formRule.action}
                  options={[
                    { value: "bypass", label: "旁路（RETURN）" },
                    { value: "reject", label: "拒绝（REJECT）" },
                  ]}
                  onChange={(event) => setFormRule({ ...formRule, action: event.currentTarget.value as NetworkRuleAction })}
                />
              </Field>
              <Field label="地址族">
                <Select
                  value={formRule.family}
                  options={[
                    { value: "both", label: "双栈" },
                    { value: "ipv4", label: "IPv4" },
                    { value: "ipv6", label: "IPv6" },
                  ]}
                  onChange={(event) => setFormRule({ ...formRule, family: event.currentTarget.value as NetworkRuleFamily })}
                />
              </Field>
            </div>

            <Field label="作用层" hint="选择规则进入哪些 iptables 链">
              <div className="flex flex-wrap gap-1.5">
                {(Object.keys(SCOPE_LABELS) as NetworkRuleScope[]).map((scope) => (
                  <ChoiceChip
                    key={scope}
                    active={formRule.scopes.includes(scope)}
                    onClick={() => setFormRule({
                      ...formRule,
                      scopes: formRule.scopes.includes(scope)
                        ? formRule.scopes.filter((value) => value !== scope)
                        : [...formRule.scopes, scope],
                    })}
                  >
                    {SCOPE_LABELS[scope]}
                  </ChoiceChip>
                ))}
              </div>
            </Field>

            <Field label="网络模式" hint="只在这些模式下生成规则">
              <div className="flex flex-wrap gap-1.5">
                {ALL_MODES.map((mode) => (
                  <ChoiceChip
                    key={mode}
                    active={formRule.modes.includes(mode)}
                    onClick={() => setFormRule({
                      ...formRule,
                      modes: formRule.modes.includes(mode)
                        ? formRule.modes.filter((value) => value !== mode)
                        : [...formRule.modes, mode],
                    })}
                  >
                    {MODE_LABELS[mode]}
                  </ChoiceChip>
                ))}
              </div>
            </Field>

            <Field label="协议">
              <Select
                value={formRule.protocol}
                options={[
                  { value: "any", label: "任意" },
                  { value: "tcp", label: "TCP" },
                  { value: "udp", label: "UDP" },
                ]}
                onChange={(event) => setFormRule({ ...formRule, protocol: event.currentTarget.value as NetworkRuleProtocol })}
              />
            </Field>

            <Field label="目标 CIDR" htmlFor="rule-dst-cidr" hint="支持 IPv4/IPv6；留空表示不按地址匹配">
              <TagInput
                id="rule-dst-cidr"
                values={formRule.match.dst_cidrs}
                onChange={(values) => setFormRule(withMatch(formRule, { ...formRule.match, dst_cidrs: values }))}
                placeholder="例如 192.168.0.0/16"
                mono
              />
            </Field>

            <Field label="目标端口" htmlFor="rule-dst-port" hint="支持单端口或区间，如 137-139">
              <TagInput
                id="rule-dst-port"
                values={formRule.match.dst_ports}
                onChange={(values) => setFormRule(withMatch(formRule, { ...formRule.match, dst_ports: values }))}
                placeholder="例如 5353"
                mono
              />
            </Field>

            <button
              type="button"
              className="flex w-full items-center justify-between rounded-xl border border-border px-3 py-2.5 text-xs font-medium text-fg-muted hover:text-fg"
              onClick={() => setAdvancedOpen((open) => !open)}
            >
              高级条件
              <ChevronDown size={15} className={`transition-transform ${advancedOpen ? "rotate-180" : ""}`} />
            </button>

            {advancedOpen ? (
              <div className="space-y-4 rounded-xl border border-border p-3">
                <Field label="源 CIDR">
                  <TagInput
                    values={formRule.match.src_cidrs}
                    onChange={(values) => setFormRule(withMatch(formRule, { ...formRule.match, src_cidrs: values }))}
                    placeholder="例如 10.0.0.0/8"
                    mono
                  />
                </Field>
                <Field label="源端口">
                  <TagInput
                    values={formRule.match.src_ports}
                    onChange={(values) => setFormRule(withMatch(formRule, { ...formRule.match, src_ports: values }))}
                    placeholder="例如 443"
                    mono
                  />
                </Field>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="源接口">
                    <TagInput
                      values={formRule.match.src_ifaces}
                      onChange={(values) => setFormRule(withMatch(formRule, { ...formRule.match, src_ifaces: values }))}
                      placeholder="例如 ap0"
                      mono
                    />
                  </Field>
                  <Field label="目标接口">
                    <TagInput
                      values={formRule.match.dst_ifaces}
                      onChange={(values) => setFormRule(withMatch(formRule, { ...formRule.match, dst_ifaces: values }))}
                      placeholder="例如 wlan0"
                      mono
                    />
                  </Field>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="UID" hint="逗号分隔；本机 OUTPUT 有效">
                    <Input
                      value={formRule.match.uids.join(",")}
                      onChange={(event) => setFormRule(withMatch(formRule, {
                        ...formRule.match,
                        uids: event.currentTarget.value.split(",").map((value) => Number(value.trim())).filter((value) => Number.isInteger(value) && value > 0),
                      }))}
                      placeholder="例如 10123"
                    />
                  </Field>
                  <Field label="GID">
                    <Input
                      value={formRule.match.gids.join(",")}
                      onChange={(event) => setFormRule(withMatch(formRule, {
                        ...formRule.match,
                        gids: event.currentTarget.value.split(",").map((value) => Number(value.trim())).filter((value) => Number.isInteger(value) && value > 0),
                      }))}
                      placeholder="例如 3005"
                    />
                  </Field>
                </div>
                <Field label="目标类型">
                  <div className="flex flex-wrap gap-1.5">
                    {(Object.keys(DST_TYPE_LABELS) as NetworkRuleDstType[]).map((type) => (
                      <ChoiceChip
                        key={type}
                        active={formRule.match.dst_types.includes(type)}
                        onClick={() => setFormRule(withMatch(formRule, {
                          ...formRule.match,
                          dst_types: formRule.match.dst_types.includes(type)
                            ? formRule.match.dst_types.filter((value) => value !== type)
                            : [...formRule.match.dst_types, type],
                        }))}
                      >
                        {DST_TYPE_LABELS[type]}
                      </ChoiceChip>
                    ))}
                  </div>
                </Field>
              </div>
            ) : null}

            <p className="rounded-xl bg-surface-muted px-3 py-2 text-[11px] leading-5 text-fg-muted">
              当前摘要：{ruleSummary(formRule)}
            </p>
          </div>
        ) : null}
      </Sheet>

      <ConfirmDialog
        open={deletingIndex !== null}
        onOpenChange={(open) => { if (!open) setDeletingIndex(null); }}
        title="删除网络规则"
        description={deletingIndex === null ? "" : `确定删除「${rules[deletingIndex]?.name}」？删除默认规则后会被记入 tombstone，升级不会自动恢复。`}
        confirmLabel="删除"
        destructive
        onConfirm={remove}
      />

    </div>
  );
}
