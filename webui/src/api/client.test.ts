import { afterEach, describe, expect, it } from "vitest";
import { ApiError, execCommand, parseEnvelope, shellQuote } from "./client";
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
