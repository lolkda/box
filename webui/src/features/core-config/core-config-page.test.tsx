import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CoreConfigPage } from "./core-config-page";
import { ToastProvider } from "../../components/ui/toast";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../../test/ksu-spawn-mock";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installMock() {
  installKsuSpawnMock((command) => {
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
    return { stdout: JSON.stringify(payload) };
  });
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
    clearKsuSpawnMock();
  });

  it("renders mihomo structured fields and raw tab", async () => {
    renderPage();
    expect(await screen.findByText("安全运行层")).toBeInTheDocument();
    expect(screen.getByText("外部控制地址")).toBeInTheDocument();
    expect(screen.getByText("高级 YAML")).toBeInTheDocument();
    const actionBar = screen.getByRole("region", { name: "核心配置操作栏" });
    expect(within(actionBar).getByRole("button", { name: /保存核心配置/ })).toBeDisabled();
  });

  it("shows the prepared find-process-mode as read-only", async () => {
    renderPage();
    const field = await screen.findByLabelText("进程匹配模式");
    expect(field).toHaveAttribute("readonly");
    expect(field).toHaveValue("always");
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
