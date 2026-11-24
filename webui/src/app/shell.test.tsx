import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AppShell } from "./shell";
import { OverviewPage } from "../features/overview/overview-page";
import { ToastProvider } from "../components/ui/toast";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../test/ksu-spawn-mock";

function installStatusMock(core = "mihomo") {
  installKsuSpawnMock((command) => {
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
    return { stdout: JSON.stringify(payload) };
  });
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
    clearKsuSpawnMock();
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

  it("does not render the top app bar or task shortcut", async () => {
    renderApp();
    expect(await screen.findByText("运行中")).toBeInTheDocument();
    expect(screen.queryByText("KernelSU WebUI")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("任务中心")).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/个任务运行中/)).not.toBeInTheDocument();
  });

  it("shows an upgrade message when ksu.spawn is unavailable", async () => {
    (window as Window & { ksu?: unknown }).ksu = { exec() {} };
    renderApp();
    expect(await screen.findByText("请升级 KernelSU")).toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "主导航" })).not.toBeInTheDocument();
  });

});
