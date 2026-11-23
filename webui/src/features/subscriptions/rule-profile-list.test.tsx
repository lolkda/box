import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ToastProvider } from "../../components/ui/toast";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../../test/ksu-spawn-mock";
import { RuleProfileList } from "./rule-profile-list";
import type { RuleProfileItem } from "../../api/types";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function item(overrides: Partial<RuleProfileItem> = {}): RuleProfileItem {
  return {
    id: "default",
    name: "默认规则",
    url_masked: "https://rules.example/***",
    active: true,
    pending: false,
    fetched_at: "2026-11-24T02:56:55+0800",
    applied_at: "2026-11-24T02:57:00+0800",
    counts: { groups: 22, rule_providers: 35, rules: 49 },
    can_delete: false,
    ...overrides,
  };
}

function installMock(initial: RuleProfileItem[]) {
  let items = initial;
  const calls: string[] = [];
  installKsuSpawnMock((command) => {
    calls.push(command);
    if (command.includes("'rule-profile' 'list'")) {
      return { stdout: JSON.stringify(envelope({ supported: true, items })) };
    }
    if (command.includes("'rule-profile' 'update'")) {
      const id = command.match(/'rule-profile' 'update' '([^']+)'/)?.[1] || "default";
      items = items.map((entry) => entry.id === id ? { ...entry, pending: true, counts: { groups: 8, rule_providers: 12, rules: 18 } } : entry);
      return { stdout: JSON.stringify(envelope(items.find((entry) => entry.id === id))) };
    }
    if (command.includes("'rule-profile' 'use'")) {
      const id = command.match(/'rule-profile' 'use' '([^']+)'/)?.[1] || "default";
      items = items.map((entry) => ({ ...entry, active: entry.id === id, pending: entry.id === id ? false : entry.pending }));
      return { stdout: JSON.stringify(envelope({ id, name: items.find((entry) => entry.id === id)?.name, changed: ["proxy-groups"], reloaded: false, updated_at: "2026-11-24T03:00:00+0800", counts: { groups: 8, rule_providers: 12, rules: 18 } })) };
    }
    if (command.includes("'rule-profile' 'add'")) {
      const name = command.match(/'rule-profile' 'add' '([^']+)'/)?.[1] || "新规则";
      const next = item({ id: "new", name, active: false, pending: false, counts: { groups: 0, rule_providers: 0, rules: 0 }, can_delete: true, fetched_at: "", applied_at: "" });
      items = [...items, next];
      return { stdout: JSON.stringify(envelope(next)) };
    }
    return { stdout: JSON.stringify(envelope({})) };
  });
  return { calls };
}

function renderList() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter>
          <RuleProfileList />
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("RuleProfileList", () => {
  afterEach(() => clearKsuSpawnMock());

  it("applies a specified profile from an existing pending snapshot", async () => {
    const user = userEvent.setup();
    const mock = installMock([
      item(),
      item({ id: "backup", name: "备用规则", active: false, pending: true, can_delete: true, counts: { groups: 8, rule_providers: 12, rules: 18 } }),
    ]);
    renderList();

    expect(await screen.findByText("默认规则")).toBeInTheDocument();
    expect(screen.getByText("使用中")).toBeInTheDocument();
    expect(screen.getByText("待应用")).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "更多操作" })[1]);
    await user.click(await screen.findByRole("menuitem", { name: "使用" }));
    expect(await screen.findByText("应用规则与策略订阅")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "仅保存" }));

    expect(await screen.findByText("规则与策略已保存")).toBeInTheDocument();
    expect(mock.calls.some((command) => command.includes("'rule-profile' 'use' 'backup'"))).toBe(true);
  });

  it("fetches before confirming use when there is no pending snapshot", async () => {
    const user = userEvent.setup();
    const mock = installMock([
      item(),
      item({ id: "fresh", name: "新规则", active: false, pending: false, can_delete: true, counts: { groups: 0, rule_providers: 0, rules: 0 }, fetched_at: "", applied_at: "" }),
    ]);
    renderList();

    expect(await screen.findByText("新规则")).toBeInTheDocument();
    await user.click(screen.getAllByRole("button", { name: "更多操作" })[1]);
    await user.click(await screen.findByRole("menuitem", { name: "使用" }));
    expect(await screen.findByText("应用规则与策略订阅")).toBeInTheDocument();
    expect(mock.calls.some((command) => command.includes("'rule-profile' 'update' 'fresh'"))).toBe(true);
  });

  it("adds a named rule profile", async () => {
    const user = userEvent.setup();
    installMock([item()]);
    renderList();

    await user.click(await screen.findByRole("button", { name: "添加规则订阅" }));
    await user.type(screen.getByLabelText("名称"), "机场 C 规则");
    await user.type(screen.getByLabelText("规则模板 URL"), "https://rules.example/c.yaml");
    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("机场 C 规则")).toBeInTheDocument();
  });
});
