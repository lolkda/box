import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AppsNetworkPage } from "./apps-network-page";
import { ToastProvider } from "../../components/ui/toast";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../../test/ksu-spawn-mock";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installMock() {
  installKsuSpawnMock((command) => {
    const payload = command.includes("'lists' 'get' '--json'")
      ? envelope({
          package: [{ uid: "0", package: "com.example.app" }],
          gid: ["1000"],
          interfaces: { allow: ["ap+", "wlan+"], ignore: [] },
          files: { package: "/tmp/package.list.cfg", gid: "/tmp/gid.list.cfg", interfaces: "/tmp/ap.list.cfg" },
        })
      : envelope({});
    return { stdout: JSON.stringify(payload) };
  });
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <AppsNetworkPage />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("AppsNetworkPage", () => {
  beforeEach(() => installMock());
  afterEach(() => {
    clearKsuSpawnMock();
  });

  it("renders package entries and interface tabs", async () => {
    renderPage();
    expect(await screen.findByText("com.example.app")).toBeInTheDocument();
    expect(screen.getByText("应用包清单")).toBeInTheDocument();
    expect(screen.getByText("网络接口")).toBeInTheDocument();
    const actionBar = screen.getByRole("region", { name: "应用与网络操作栏" });
    expect(within(actionBar).getByRole("button", { name: "撤销" })).toBeDisabled();
    expect(within(actionBar).getByRole("button", { name: /保存清单/ })).toBeDisabled();
  });
});
