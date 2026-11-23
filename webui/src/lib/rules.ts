import type { ProxyGroup, RuleProvider } from "../api/types";

export type RuleMode = "structured" | "raw";

export interface RuleDraft {
  mode: RuleMode;
  type: string;
  value: string;
  target: string;
  options: string[];
  raw: string;
}

export interface RuleTypeMeta {
  value: string;
  label: string;
  valueLabel: string;
  valuePlaceholder: string;
  valueRequired: boolean;
}

export const ruleTypes: RuleTypeMeta[] = [
  { value: "DOMAIN", label: "DOMAIN", valueLabel: "域名", valuePlaceholder: "example.com", valueRequired: true },
  { value: "DOMAIN-SUFFIX", label: "DOMAIN-SUFFIX", valueLabel: "域名后缀", valuePlaceholder: "example.com", valueRequired: true },
  { value: "DOMAIN-KEYWORD", label: "DOMAIN-KEYWORD", valueLabel: "关键词", valuePlaceholder: "google", valueRequired: true },
  { value: "IP-CIDR", label: "IP-CIDR", valueLabel: "IPv4 网段", valuePlaceholder: "192.168.1.0/24", valueRequired: true },
  { value: "IP-CIDR6", label: "IP-CIDR6", valueLabel: "IPv6 网段", valuePlaceholder: "fd00::/8", valueRequired: true },
  { value: "GEOIP", label: "GEOIP", valueLabel: "国家/地区代码", valuePlaceholder: "CN", valueRequired: true },
  { value: "GEOSITE", label: "GEOSITE", valueLabel: "站点分类", valuePlaceholder: "cn", valueRequired: true },
  { value: "RULE-SET", label: "RULE-SET", valueLabel: "规则集", valuePlaceholder: "选择规则集", valueRequired: true },
  { value: "DST-PORT", label: "DST-PORT", valueLabel: "目标端口", valuePlaceholder: "443", valueRequired: true },
  { value: "PROCESS-NAME", label: "PROCESS-NAME", valueLabel: "进程名", valuePlaceholder: "com.example.app", valueRequired: true },
  { value: "MATCH", label: "MATCH", valueLabel: "无需内容", valuePlaceholder: "", valueRequired: false },
];

const simpleTypes = new Set(ruleTypes.map((item) => item.value));

export function createRuleDraft(): RuleDraft {
  return { mode: "structured", type: "DOMAIN", value: "", target: "", options: [], raw: "" };
}

export function parseRule(rule: string): RuleDraft {
  const text = rule.trim();
  const fields = text.split(",").map((field) => field.trim());
  const type = fields[0] || "";
  if (simpleTypes.has(type)) {
    if (type === "MATCH") {
      return { mode: "structured", type, value: "", target: fields[1] || "", options: [], raw: text };
    }
    return {
      mode: "structured",
      type,
      value: fields[1] || "",
      target: fields[2] || "",
      options: fields.slice(3).filter(Boolean),
      raw: text,
    };
  }
  return { mode: "raw", type, value: "", target: "", options: [], raw: text };
}

export function serializeRule(draft: RuleDraft): string {
  if (draft.mode === "raw") return draft.raw.trim();
  if (draft.type === "MATCH") return `MATCH,${draft.target.trim()}`;
  const options = draft.options.length ? `,${draft.options.join(",")}` : "";
  return `${draft.type},${draft.value.trim()},${draft.target.trim()}${options}`;
}

export function validateRuleDraft(
  draft: RuleDraft,
  groups: ProxyGroup[],
  providers: Record<string, RuleProvider>,
): string | null {
  if (draft.mode === "raw") {
    return draft.raw.trim() ? null : "规则内容不能为空";
  }
  const meta = ruleTypes.find((item) => item.value === draft.type);
  if (!meta) return "未知规则类型";
  if (meta.valueRequired && !draft.value.trim()) return `请填写${meta.valueLabel}`;
  if (!draft.target.trim()) return "请选择目标策略组";
  const groupNames = groups.map((group) => group.name);
  const builtins = ["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE"];
  if (!groupNames.includes(draft.target.trim()) && !builtins.includes(draft.target.trim())) {
    return `策略组不存在: ${draft.target}`;
  }
  if (draft.type === "RULE-SET" && !(draft.value.trim() in providers)) {
    return `规则集不存在: ${draft.value}`;
  }
  return null;
}

export function ruleTargets(rule: string): string[] {
  const draft = parseRule(rule);
  return draft.mode === "structured" && draft.target ? [draft.target] : [];
}

export function ruleUsesProvider(rule: string, providerName: string): boolean {
  const draft = parseRule(rule);
  return draft.mode === "structured" && draft.type === "RULE-SET" && draft.value === providerName;
}
