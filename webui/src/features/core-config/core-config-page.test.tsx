import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CoreConfigPage } from "./core-config-page";
import { ToastProvider } from "../../components/ui/toast";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installMock() {
  const mock = {
    exec(command: string, _options: string, callbackName: string) {
      let payload: unknown = envelope({});
      if (command.includes("'status' '--json'")) {
        payload = envelope({ running: true, pid: 1, core: "mihomo", mode: "tproxy", config: "config.yaml", config_exists: true, subscriptions: 0, active_tasks: 0 });
      } else if (command.includes("'core' 'get' '--json'")) {
        payload = envelope({
          core: "mihomo",
          file: "/data/adb/box/mihomo/config.yaml",
          structured: {
            mode: "rule",
            "log-level": "error",
            "allow-lan": true,
            "external-controller": "0.0.0.0:9090",
            "external-ui": "./dashboard",
            "profile.store-selected": true,
            "profile.store-fake-ip": true,
            "unified-delay": false,
            "tcp-concurrent": true,
            "find-process-mode": "always",
          },
          raw: "mode: rule\n",
        });
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
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <CoreConfigPage />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("CoreConfigPage", () => {
  beforeEach(() => installMock());
  afterEach(() => {
    delete (window as Window & { ksu?: unknown }).ksu;
    delete (globalThis as unknown as { ksu?: unknown }).ksu;
  });

  it("renders mihomo structured fields and raw tab", async () => {
    renderPage();
    expect(await screen.findByText("安全运行层")).toBeInTheDocument();
    expect(screen.getByText("外部控制地址")).toBeInTheDocument();
    expect(screen.getByText("高级 YAML")).toBeInTheDocument();
  });

  it("keeps external-ui readonly and exposes dashboard update", async () => {
    renderPage();
    const externalUi = await screen.findByLabelText("外部 UI 路径");
    expect(externalUi).toHaveAttribute("readonly");
    expect(externalUi).toHaveValue("./dashboard");

    fireEvent.click(screen.getByRole("button", { name: "更新面板" }));
    expect(await screen.findByText("已开始更新面板")).toBeInTheDocument();
  });
});
