import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AppsNetworkPage } from "./apps-network-page";
import { ToastProvider } from "../../components/ui/toast";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

function installMock() {
  const mock = {
    exec(command: string, _options: string, callbackName: string) {
      const payload = command.includes("'lists' 'get' '--json'")
        ? envelope({
            package: [{ uid: "0", package: "com.example.app" }],
            gid: ["1000"],
            interfaces: { allow: ["ap+", "wlan+"], ignore: [] },
            files: { package: "/tmp/package.list.cfg", gid: "/tmp/gid.list.cfg", interfaces: "/tmp/ap.list.cfg" },
          })
        : envelope({});
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
        <AppsNetworkPage />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("AppsNetworkPage", () => {
  beforeEach(() => installMock());
  afterEach(() => {
    delete (window as Window & { ksu?: unknown }).ksu;
    delete (globalThis as unknown as { ksu?: unknown }).ksu;
  });

  it("renders package entries and interface tabs", async () => {
    renderPage();
    expect(await screen.findByText("com.example.app")).toBeInTheDocument();
    expect(screen.getByText("应用包清单")).toBeInTheDocument();
    expect(screen.getByText("网络接口")).toBeInTheDocument();
  });
});
