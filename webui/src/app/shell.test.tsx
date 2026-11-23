import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AppShell } from "./shell";
import { OverviewPage } from "../features/overview/overview-page";
import { ToastProvider } from "../components/ui/toast";

function installStatusMock(core = "mihomo") {
  const mock = {
    exec(command: string, _options: string, callbackName: string) {
      let payload;
      if (command.includes("status") && command.includes("--json")) {
        payload = {
          ok: true,
          data: { running: true, pid: 1234, core, mode: "tproxy", config: "config.yaml", config_exists: true, subscriptions: 2, active_tasks: 0 },
          meta: { version: 1 },
        };
      } else if (command.includes("task") && command.includes("list")) {
        payload = { ok: true, data: { items: [] }, meta: { version: 1 } };
      } else if (command.includes("logs")) {
        payload = { ok: true, data: { file: "/tmp/runs.log", lines: ["2026-11-23 08:00:00 [Info]: ready"] }, meta: { version: 1 } };
      } else {
        payload = { ok: true, data: {}, meta: { version: 1 } };
      }
      window.setTimeout(() => {
        (window as unknown as Record<string, (errno: number, stdout: string, stderr: string) => void>)[callbackName]?.(0, JSON.stringify(payload), "");
      }, 5);
    },
  };
  (window as Window & { ksu?: unknown }).ksu = mock;
  (globalThis as unknown as { ksu?: unknown }).ksu = mock;
}

function renderApp() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const router = createMemoryRouter(
    [{ path: "/", element: <AppShell />, children: [{ index: true, element: <OverviewPage /> }] }],
    { initialEntries: ["/"] },
  );
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <RouterProvider router={router} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("AppShell", () => {
  beforeEach(() => installStatusMock());
  afterEach(() => {
    delete (window as Window & { ksu?: unknown }).ksu;
  });

  it("renders the bottom navigation and overview status", async () => {
    renderApp();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeInTheDocument();
    for (const label of ["概览", "订阅", "策略", "规则", "更多"]) {
      expect(screen.getAllByText(label).length).toBeGreaterThan(0);
    }
    expect(await screen.findByText("运行中")).toBeInTheDocument();
    expect(await screen.findByText("mihomo · tproxy · PID 1234")).toBeInTheDocument();
  });

  it("blocks mihomo-only pages when the core is sing-box", async () => {
    installStatusMock("sing-box");
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const router = createMemoryRouter(
      [{ path: "/", element: <AppShell />, children: [{ path: "rules", element: <div>rules content</div> }] }],
      { initialEntries: ["/rules"] },
    );
    render(
      <QueryClientProvider client={client}>
        <ToastProvider>
          <RouterProvider router={router} />
        </ToastProvider>
      </QueryClientProvider>,
    );
    expect(await screen.findByText("规则 仅支持 mihomo")).toBeInTheDocument();
    expect(screen.queryByText("rules content")).not.toBeInTheDocument();
  });
});
