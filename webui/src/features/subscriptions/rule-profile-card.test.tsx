import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ToastProvider } from "../../components/ui/toast";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../../test/ksu-spawn-mock";
import { RuleProfileCard } from "./rule-profile-card";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installRuleProfileMock(options: { configured?: boolean; supported?: boolean } = {}) {
  const supported = options.supported ?? true;
  let configured = options.configured ?? false;
  let counts = { groups: 22, rule_providers: 33, rules: 49 };
  let appliedReload: boolean | null = null;

  installKsuSpawnMock((command) => {
    let data: unknown = {};
    if (command.includes("'rule-profile' 'get'")) {
      data = {
        supported,
        configured,
        url_masked: configured ? "https://rules.example/***" : "",
        updated_at: configured ? "2026-11-24T01:00:00+0800" : "",
        counts,
      };
    } else if (command.includes("'rule-profile' 'url'")) {
      data = { url: "https://rules.example/config.yaml" };
    } else if (command.includes("'rule-profile' 'set'")) {
      configured = true;
      data = {
        supported: true,
        configured,
        url_masked: "https://rules.example/***",
        updated_at: "",
        counts,
      };
    } else if (command.includes("'rule-profile' 'fetch'")) {
      counts = { groups: 2, rule_providers: 1, rules: 3 };
      data = { url_masked: "https://rules.example/***", fetched_at: "2026-11-24T01:01:00+0800", counts };
    } else if (command.includes("'rule-profile' 'apply'")) {
      appliedReload = command.includes("--reload");
      data = { changed: ["proxy-groups", "rule-providers", "rules"], reloaded: appliedReload, updated_at: "2026-11-24T01:02:00+0800", counts };
    } else if (command.includes("'rule-profile' 'clear'")) {
      configured = false;
      data = { cleared: true };
    }
    return { stdout: JSON.stringify(envelope(data)) };
  });

  return {
    applied: () => appliedReload,
    setCounts: (next: typeof counts) => { counts = next; },
  };
}

function renderCard() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter>
          <RuleProfileCard />
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("RuleProfileCard", () => {
  afterEach(() => {
    clearKsuSpawnMock();
  });

  it("configures a global rule profile URL", async () => {
    installRuleProfileMock();
    const user = userEvent.setup();
    renderCard();

    expect(await screen.findByText("规则与策略订阅")).toBeInTheDocument();
    expect(screen.getByText("未配置")).toBeInTheDocument();
    expect(screen.getByText("22")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "配置规则订阅" }));
    await user.type(screen.getByLabelText("规则模板 URL"), "https://rules.example/config.yaml");
    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("已配置")).toBeInTheDocument();
    expect(await screen.findByText("规则订阅地址已保存")).toBeInTheDocument();
  });

  it("downloads and applies only the three managed sections", async () => {
    const mock = installRuleProfileMock({ configured: true });
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole("button", { name: "更新规则与策略" }));
    expect(await screen.findByText("应用规则与策略订阅")).toBeInTheDocument();
    expect(screen.getByText(/整体替换当前策略组/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "仅保存" }));
    expect(await screen.findByText("规则与策略已保存")).toBeInTheDocument();
    expect(mock.applied()).toBe(false);
  });

  it("can choose save and reload", async () => {
    const mock = installRuleProfileMock({ configured: true });
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole("button", { name: "更新规则与策略" }));
    await user.click(await screen.findByRole("button", { name: "保存并重载" }));

    expect(await screen.findByText("规则与策略已保存并重载")).toBeInTheDocument();
    expect(mock.applied()).toBe(true);
  });
});
