import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ToastProvider } from "../../components/ui/toast";
import { NodeSubscriptionsPage } from "./node-subscriptions-page";
import { RuleProfilePage } from "./rule-profile-page";
import { SubscriptionsPage } from "./subscriptions-page";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../../test/ksu-spawn-mock";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installSubscriptionMock() {
  const subscriptions = [
    { name: "机场 A", url_masked: "https://example.com/***", provider: "downloaded", provider_size: 1024, provider_count: 0, updated_at: "2026-11-23T07:00:00+0800" },
  ];
  installKsuSpawnMock((command) => {
    let payload: unknown;
    if (command.includes("'rule-profile' 'get'")) {
      payload = envelope({ supported: true, configured: false, url_masked: "", updated_at: "", counts: { groups: 0, rule_providers: 0, rules: 0 } });
    } else if (command.includes("'sub' 'list'")) {
      payload = envelope({ items: subscriptions });
    } else if (command.includes("'sub' 'add'")) {
      subscriptions.push({ name: "测试新增", url_masked: "https://new.example.com/***", provider: "missing", provider_size: 0, provider_count: 0, updated_at: "" });
      payload = envelope({ name: "测试新增", url_masked: "https://new.example.com/***" });
    } else {
      payload = envelope({ items: subscriptions });
    }
    return { stdout: JSON.stringify(payload) };
  });
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <MemoryRouter initialEntries={["/subscriptions"]}>
          <Routes>
            <Route path="/subscriptions" element={<SubscriptionsPage />}>
              <Route index element={<NodeSubscriptionsPage />} />
              <Route path="rule-profile" element={<RuleProfilePage />} />
            </Route>
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("SubscriptionsPage", () => {
  beforeEach(() => installSubscriptionMock());
  afterEach(() => {
    clearKsuSpawnMock();
  });

  it("switches between node subscriptions and the rule profile tab", async () => {
    const user = userEvent.setup();
    renderPage();

    expect(await screen.findByText("机场 A")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "添加订阅" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "规则与策略" })).toHaveAttribute("href", "/subscriptions/rule-profile");
    await user.click(screen.getByRole("link", { name: "规则与策略" }));
    expect(await screen.findByText("规则与策略订阅")).toBeInTheDocument();
    expect(screen.queryByText("机场 A")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "添加订阅" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "更新全部订阅" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("link", { name: "节点订阅" }));
    expect(await screen.findByText("机场 A")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "添加订阅" })).toBeInTheDocument();
  });

  it("adds a subscription through the sheet", async () => {
    const user = userEvent.setup();
    renderPage();

    expect(await screen.findByText("机场 A")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "添加订阅" }));
    await user.type(screen.getByLabelText("名字"), "测试新增");
    await user.type(screen.getByLabelText(/订阅地址/), "https://new.example.com/sub");
    await user.click(screen.getByRole("button", { name: "添加" }));

    expect(await screen.findByText("测试新增")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
