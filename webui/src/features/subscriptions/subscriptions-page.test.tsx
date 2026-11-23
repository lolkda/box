import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ToastProvider } from "../../components/ui/toast";
import { SubscriptionsPage } from "./subscriptions-page";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installSubscriptionMock() {
  const subscriptions = [
    { name: "机场 A", url_masked: "https://example.com/***", provider: "downloaded", provider_size: 1024, updated_at: "2026-11-23T07:00:00+0800" },
  ];
  const mock = {
    exec(command: string, _options: string, callbackName: string) {
      let payload: unknown;
      if (command.includes("'sub' 'list'")) {
        payload = envelope({ items: subscriptions });
      } else if (command.includes("'sub' 'add'")) {
        subscriptions.push({ name: "测试新增", url_masked: "https://new.example.com/***", provider: "missing", provider_size: 0, updated_at: "" });
        payload = envelope({ name: "测试新增", url_masked: "https://new.example.com/***" });
      } else {
        payload = envelope({ items: subscriptions });
      }
      window.setTimeout(() => {
        (window as unknown as Record<string, (errno: number, stdout: string, stderr: string) => void>)[callbackName]?.(0, JSON.stringify(payload), "");
      }, 5);
    },
  };
  (window as Window & { ksu?: unknown }).ksu = mock;
  (globalThis as unknown as { ksu?: unknown }).ksu = mock;
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const router = createMemoryRouter([{ path: "/subscriptions", element: <SubscriptionsPage /> }], { initialEntries: ["/subscriptions"] });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <RouterProvider router={router} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("SubscriptionsPage", () => {
  beforeEach(() => installSubscriptionMock());
  afterEach(() => {
    delete (window as Window & { ksu?: unknown }).ksu;
    delete (globalThis as unknown as { ksu?: unknown }).ksu;
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
