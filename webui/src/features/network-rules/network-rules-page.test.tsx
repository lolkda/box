import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NetworkRulesPage } from "./network-rules-page";
import { ToastProvider } from "../../components/ui/toast";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../../test/ksu-spawn-mock";

function envelope(data: unknown) {
  return { ok: true, data, meta: { version: 1 } };
}

const rule = {
  id: "default-private-v4",
  name: "IPv4 私网",
  enabled: true,
  action: "bypass",
  scopes: ["local", "forward"],
  modes: ["tun", "tproxy", "redirect", "mixed", "enhance"],
  family: "ipv4",
  protocol: "any",
  match: { dst_cidrs: ["192.168.0.0/16"], src_cidrs: [], dst_ports: [], src_ports: [], src_ifaces: [], dst_ifaces: [], uids: [], gids: [], dst_types: [] },
  origin: "default",
};

function installMock() {
  installKsuSpawnMock((command) => {
    if (command.includes("'network-rules' 'get'")) {
      return {
        stdout: JSON.stringify(envelope({
          schema_version: 1,
          revision: 3,
          defaults_version: "test",
          rules: [rule],
          tombstones: [],
          system: [{ id: "system-core", name: "核心旁路", description: "只读保护", scopes: ["local"], readOnly: true }],
          warnings: [],
        })),
      };
    }
    return { stdout: JSON.stringify(envelope({})) };
  });
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <NetworkRulesPage />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("NetworkRulesPage", () => {
  beforeEach(() => {
    window.localStorage.clear();
    installMock();
  });
  afterEach(() => clearKsuSpawnMock());

  it("renders read-only protection and editable rows", async () => {
    renderPage();
    expect(await screen.findByText("核心旁路")).toBeInTheDocument();
    expect(screen.getByText("IPv4 私网")).toBeInTheDocument();
    expect(screen.getByText("只读")).toBeInTheDocument();
    const actionBar = screen.getByRole("region", { name: "网络规则操作栏" });
    expect(within(actionBar).getByRole("button", { name: "预览变更" })).toBeDisabled();
    expect(within(actionBar).queryByRole("button", { name: "清空全部" })).not.toBeInTheDocument();
    expect(screen.queryByText("清空全部规则")).not.toBeInTheDocument();
  });

  it("keeps an edited draft locally before applying", async () => {
    renderPage();
    await screen.findByText("IPv4 私网");
    await userEvent.click(screen.getByRole("button", { name: "新增网络规则" }));
    await userEvent.clear(screen.getByLabelText("名称"));
    await userEvent.type(screen.getByLabelText("名称"), "测试草稿");
    const cidr = screen.getByLabelText("目标 CIDR");
    await userEvent.type(cidr, "203.0.113.0/24{enter}");
    await userEvent.click(screen.getByRole("button", { name: "保存到草稿" }));
    expect(await screen.findByText("测试草稿")).toBeInTheDocument();
    expect(screen.getByText("有未应用修改")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "预览变更" })).toBeEnabled();
    expect(window.localStorage.getItem("box.network-rules.draft.v1")).toContain("203.0.113.0/24");
  });

  it("opens the row editor", async () => {
    renderPage();
    await screen.findByText("IPv4 私网");
    await userEvent.click(screen.getByRole("button", { name: "新增网络规则" }));
    expect(screen.getByText("新增网络规则")).toBeInTheDocument();
    expect(screen.getByLabelText("名称")).toBeInTheDocument();
  });
});
