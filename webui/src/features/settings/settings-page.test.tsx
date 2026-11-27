import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import type { SettingsData } from "../../api/types";
import { queryKeys } from "../../api/queries";
import { ToastProvider } from "../../components/ui/toast";
import { SettingsPage } from "./settings-page";

const settingsData = {
  boot_auto_start: "false",
  auto_modify_config: "true",
  network_mode: "tproxy",
  find_process_mode: "off",
  tproxy_port: "9898",
  redir_port: "9797",
  subscription_keep_last_good: "true",
  subscription_user_agent: "clash-verge/v2.4.5",
  interva_update: "0 0,6,12,18 * * *",
  sensitive: { githubtoken: { set: false } },
} satisfies SettingsData;

function renderPage() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  client.setQueryData(queryKeys.settings, settingsData);
  const router = createMemoryRouter([{ path: "/settings", element: <SettingsPage /> }], {
    initialEntries: ["/settings"],
  });

  return render(
    <QueryClientProvider client={client}>
      <ToastProvider>
        <RouterProvider router={router} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("SettingsPage", () => {
  it("removes the internal path note and keeps the action bar mounted when clean", async () => {
    renderPage();

    const actionBar = await screen.findByRole("region", { name: "设置操作栏" });
    expect(screen.queryByText("内部路径与常量")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /恢复运行值/ })).not.toBeInTheDocument();
    expect(within(actionBar).queryByRole("button", { name: "撤销" })).not.toBeInTheDocument();
    expect(within(actionBar).getByRole("button", { name: /保存更改/ })).toBeDisabled();
  });

  it("enables the action bar after an edit and restores the saved values", async () => {
    const user = userEvent.setup();
    renderPage();

    const networkMode = await screen.findByLabelText("网络模式");
    await user.selectOptions(networkMode, "tun");

    const actionBar = screen.getByRole("region", { name: "设置操作栏" });
    const undo = within(actionBar).getByRole("button", { name: "撤销" });
    const save = within(actionBar).getByRole("button", { name: /保存更改/ });

    await waitFor(() => expect(save).toBeEnabled());
    expect(undo).toBeEnabled();

    await user.click(undo);
    await waitFor(() => expect(save).toBeDisabled());
    expect(within(actionBar).queryByRole("button", { name: "撤销" })).not.toBeInTheDocument();
    expect(networkMode).toHaveValue("tproxy");
  });
});
