import { afterEach, describe, expect, it } from "vitest";
import { ApiError, execCommand, listInstalledPackages, parseEnvelope, resolvePackageBridge, shellQuote } from "./client";
import { clearKsuSpawnMock, installKsuSpawnMock } from "../test/ksu-spawn-mock";

describe("parseEnvelope", () => {
  it("returns data for a successful envelope", () => {
    const data = parseEnvelope<{ running: boolean }>(
      '{"ok":true,"data":{"running":true},"meta":{"version":1}}',
      0,
      "",
    );
    expect(data).toEqual({ running: true });
  });

  it("throws a typed error for a failed envelope", () => {
    expect(() =>
      parseEnvelope(
        '{"ok":false,"error":{"code":"E_VALIDATION","message":"配置非法","detail":"rules"},"meta":{"version":1}}',
        1,
        "",
      ),
    ).toThrowError(expect.objectContaining({ code: "E_VALIDATION", message: "配置非法" }));
  });

  it("throws a protocol error for non-JSON output", () => {
    expect(() => parseEnvelope("not json", 1, "shell failed")).toThrowError(ApiError);
  });
});

describe("shellQuote", () => {
  it("quotes normal values", () => {
    expect(shellQuote("hello world")).toBe("'hello world'");
  });

  it("escapes single quotes", () => {
    expect(shellQuote("it's ok")).toBe("'it'\\''s ok'");
  });
});

describe("execCommand", () => {
  afterEach(() => {
    clearKsuSpawnMock();
  });

  it("collects stdout and stderr and preserves the exit code", async () => {
    installKsuSpawnMock(() => ({ errno: 3, stdout: "first line\nsecond line", stderr: "warning" }));

    await expect(execCommand("/data/adb/box/scripts/box.tool status --json")).resolves.toEqual({
      errno: 3,
      stdout: "first line\nsecond line",
      stderr: "warning",
    });
  });

  it("rejects when the KernelSU spawn bridge is unavailable", async () => {
    (window as Window & { ksu?: unknown }).ksu = { exec() {} };

    await expect(execCommand("true")).rejects.toMatchObject({ code: "E_UNSUPPORTED" });
  });
});


describe("package bridge", () => {
  const originalKsu = (window as Window & { ksu?: unknown }).ksu;

  afterEach(() => {
    if (originalKsu === undefined) delete (window as Window & { ksu?: unknown }).ksu;
    else (window as Window & { ksu?: unknown }).ksu = originalKsu;
  });

  it("uses the new pm bridge when available", async () => {
    const pm = {
      listPackages: () => JSON.stringify(["com.example.app", "com.example.app"]),
      getPackagesInfo: () => JSON.stringify([{ packageName: "com.example.app", appLabel: "示例", uid: 10001, isSystem: false }]),
    };
    (window as Window & { ksu?: unknown }).ksu = { pm: () => pm };

    expect(resolvePackageBridge()?.kind).toBe("pm");
    await expect(listInstalledPackages()).resolves.toEqual([
      { packageName: "com.example.app", appLabel: "示例", versionName: "", isSystem: false, uid: 10001 },
    ]);
  });

  it("falls back to the legacy listPackages/getPackagesInfo bridge", async () => {
    (window as Window & { ksu?: unknown }).ksu = {
      listPackages: () => JSON.stringify(["com.android.settings"]),
      getPackagesInfo: () => JSON.stringify([{ packageName: "com.android.settings", appLabel: "设置", isSystem: true }]),
    };

    expect(resolvePackageBridge()?.kind).toBe("legacy");
    await expect(listInstalledPackages()).resolves.toEqual([
      { packageName: "com.android.settings", appLabel: "设置", versionName: "", isSystem: true, uid: null },
    ]);
  });

  it("reports unavailable instead of a fake empty list", async () => {
    (window as Window & { ksu?: unknown }).ksu = {};

    await expect(listInstalledPackages()).rejects.toMatchObject({ code: "PACKAGE_BRIDGE_UNAVAILABLE" });
  });

  it("reports malformed bridge data", async () => {
    (window as Window & { ksu?: unknown }).ksu = {
      listPackages: () => "not json",
      getPackagesInfo: () => "[]",
    };

    await expect(listInstalledPackages()).rejects.toMatchObject({ code: "PACKAGE_BRIDGE_BAD_RESPONSE" });
  });
});
