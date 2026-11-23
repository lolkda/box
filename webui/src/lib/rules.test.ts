import { describe, expect, it } from "vitest";
import type { ProxyGroup, RuleProvider } from "../api/types";
import { parseRule, serializeRule, validateRuleDraft } from "./rules";

const groups: ProxyGroup[] = [{ name: "国外代理", type: "select" }, { name: "国内直连", type: "select" }];
const providers: Record<string, RuleProvider> = {
  proxy_domain: { type: "http", behavior: "domain", format: "mrs", interval: 86400, path: "./proxy.mrs" },
};

describe("rule conversion", () => {
  it("parses a structured rule and round-trips it", () => {
    const draft = parseRule("DOMAIN,example.com,国外代理");
    expect(draft.mode).toBe("structured");
    expect(draft.value).toBe("example.com");
    expect(serializeRule(draft)).toBe("DOMAIN,example.com,国外代理");
  });

  it("keeps complex rules in raw mode", () => {
    const draft = parseRule("OR,((RULE-SET,a),(RULE-SET,b)),国外代理");
    expect(draft.mode).toBe("raw");
    expect(serializeRule(draft)).toContain("OR,((RULE-SET,a)");
  });

  it("validates RULE-SET provider references", () => {
    const draft = parseRule("RULE-SET,missing,国外代理");
    expect(validateRuleDraft(draft, groups, providers)).toContain("规则集不存在");
  });

  it("accepts a valid RULE-SET rule", () => {
    const draft = parseRule("RULE-SET,proxy_domain,国外代理");
    expect(validateRuleDraft(draft, groups, providers)).toBeNull();
  });
});
