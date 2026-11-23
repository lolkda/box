import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PackagePicker } from "./package-picker";

const originalKsu = (window as Window & { ksu?: unknown }).ksu;

afterEach(() => {
  if (originalKsu === undefined) delete (window as Window & { ksu?: unknown }).ksu;
  else (window as Window & { ksu?: unknown }).ksu = originalKsu;
});

describe("PackagePicker", () => {
  it("renders app labels from the legacy KernelSU 3.3.1 bridge", async () => {
    const user = userEvent.setup();
    (window as Window & { ksu?: unknown }).ksu = {
      listPackages: () => JSON.stringify(["com.android.settings", "com.example.proxy"]),
      getPackagesInfo: () => JSON.stringify([
        { packageName: "com.android.settings", appLabel: "设置" },
        { packageName: "com.example.proxy", appLabel: "Proxy 工具" },
      ]),
    };
    const onChange = vi.fn();

    render(<PackagePicker open onOpenChange={() => undefined} values={[]} onChange={onChange} />);

    expect(await screen.findByText("Proxy 工具")).toBeInTheDocument();
    expect(screen.getByText("com.example.proxy")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Proxy 工具/ }));
    await user.click(screen.getByRole("button", { name: "应用选择（1）" }));
    expect(onChange).toHaveBeenCalledWith(["com.example.proxy"]);
  });

  it("re-checks the bridge when the picker opens", async () => {
    (window as Window & { ksu?: unknown }).ksu = {};
    const onChange = vi.fn();
    const { rerender } = render(<PackagePicker open={false} onOpenChange={() => undefined} values={[]} onChange={onChange} />);

    (window as Window & { ksu?: unknown }).ksu = {
      listPackages: () => JSON.stringify(["com.android.settings"]),
      getPackagesInfo: () => JSON.stringify([{ packageName: "com.android.settings", appLabel: "设置" }]),
    };
    rerender(<PackagePicker open onOpenChange={() => undefined} values={[]} onChange={onChange} />);

    expect(await screen.findByText("设置")).toBeInTheDocument();
  });

  it("shows an empty state instead of an unavailable API warning", async () => {
    (window as Window & { ksu?: unknown }).ksu = {
      listPackages: () => "[]",
      getPackagesInfo: () => "[]",
    };

    render(<PackagePicker open onOpenChange={() => undefined} values={[]} onChange={() => undefined} />);

    expect(await screen.findByText(/没有读取到应用/)).toBeInTheDocument();
    expect(screen.queryByText(/未提供包列表接口/)).not.toBeInTheDocument();
  });

  it("allows manual package input when the bridge is unavailable", async () => {
    const user = userEvent.setup();
    (window as Window & { ksu?: unknown }).ksu = {};
    const onChange = vi.fn();

    render(<PackagePicker open onOpenChange={() => undefined} values={[]} onChange={onChange} />);

    expect(await screen.findByText(/未提供包列表接口/)).toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "手动输入包名" }), "com.example.manual");
    await user.click(screen.getByRole("button", { name: "添加" }));
    await user.click(screen.getByRole("button", { name: "应用选择（1）" }));
    expect(onChange).toHaveBeenCalledWith(["com.example.manual"]);
  });

  it("rejects an invalid manual package name", async () => {
    const user = userEvent.setup();
    (window as Window & { ksu?: unknown }).ksu = {};

    render(<PackagePicker open onOpenChange={() => undefined} values={[]} onChange={() => undefined} />);

    await user.type(await screen.findByRole("textbox", { name: "手动输入包名" }), "not a package");
    await user.click(screen.getByRole("button", { name: "添加" }));
    expect(screen.getByText(/合法的 Android 包名/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "应用选择（0）" })).toBeInTheDocument();
  });
});
