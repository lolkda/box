import type {
  ApiEnvelope,
  AppListsData,
  AppListsPayload,
  ConfigData,
  CoreConfigData,
  MihomoStructuredConfig,
  LogsData,
  PackageInfo,
  RuleProfileApplied,
  RuleProfileData,
  RuleProfileFetched,
  RuleProfileItem,
  RuleProfileListData,
  ServiceAction,
  SettingsData,
  StatusData,
  Subscription,
  SubscriptionDetail,
  TaskInfo,
  TaskKind,
  TaskLogsData,
} from "./types";

export const TOOL = "/data/adb/box/bin/boxctl";

let tempCounter = 0;

export class ApiError extends Error {
  readonly code: string;
  readonly detail: string;

  constructor(code: string, message: string, detail = "") {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.detail = detail;
  }
}

export function shellQuote(value: string): string {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

interface KsuBridge {
  spawn?: (command: string, args: string, options: string, callbackName: string) => void;
}

type SpawnListener = (...args: any[]) => void;

class SpawnStdio {
  private readonly listeners: Record<string, SpawnListener[]> = {};

  on(event: string, listener: SpawnListener): void {
    (this.listeners[event] ||= []).push(listener);
  }

  emit(event: string, ...args: any[]): void {
    for (const listener of this.listeners[event] || []) listener(...args);
  }
}

class SpawnChildProcess {
  private readonly listeners: Record<string, SpawnListener[]> = {};
  readonly stdout = new SpawnStdio();
  readonly stderr = new SpawnStdio();

  on(event: string, listener: SpawnListener): void {
    (this.listeners[event] ||= []).push(listener);
  }

  emit(event: string, ...args: any[]): void {
    for (const listener of this.listeners[event] || []) listener(...args);
  }
}

function getKsuBridge(): KsuBridge | undefined {
  if (typeof window === "undefined") return undefined;
  return (window as Window & { ksu?: KsuBridge }).ksu;
}

export function isKernelsuAvailable(): boolean {
  return typeof getKsuBridge() !== "undefined";
}

export function supportsAsyncExec(): boolean {
  return typeof getKsuBridge()?.spawn === "function";
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

let spawnCallbackCounter = 0;

export async function execCommand(
  command: string,
  timeoutMs = 0,
): Promise<{ errno: number; stdout: string; stderr: string }> {
  const bridge = getKsuBridge();
  if (!bridge) {
    throw new ApiError("E_UNAVAILABLE", "KernelSU API 不可用，请从 KernelSU 模块页打开");
  }
  if (typeof bridge.spawn !== "function") {
    throw new ApiError("E_UNSUPPORTED", "KernelSU Manager 版本过低，请升级到 3.3 或更高版本");
  }
  const spawnCommand = bridge.spawn.bind(bridge);

  return new Promise((resolve, reject) => {
    const callbackName = `box_exec_callback_${Date.now()}_${spawnCallbackCounter++}`;
    const child = new SpawnChildProcess();
    const stdout: string[] = [];
    const stderr: string[] = [];
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const cleanup = () => {
      if (timer !== undefined) clearTimeout(timer);
      delete (window as unknown as Record<string, SpawnChildProcess>)[callbackName];
    };
    const fail = (error: unknown, keepCallback = false) => {
      if (settled) return;
      settled = true;
      if (!keepCallback) cleanup();
      reject(error instanceof ApiError ? error : new ApiError("E_EXEC", "执行命令失败", errorText(error)));
    };

    child.stdout.on("data", (data: string) => stdout.push(String(data)));
    child.stderr.on("data", (data: string) => stderr.push(String(data)));
    child.on("exit", (code: number | string | null) => {
      if (settled) {
        cleanup();
        return;
      }
      settled = true;
      cleanup();
      const numericCode = typeof code === "number" ? code : Number(code);
      resolve({
        errno: Number.isFinite(numericCode) ? numericCode : 0,
        stdout: stdout.join("\n"),
        stderr: stderr.join("\n"),
      });
    });
    child.on("error", (error: unknown) => {
      // KernelSU also emits an error after a non-zero exit; the exit handler wins.
      if (!settled) fail(error);
    });

    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        // Keep the callback object alive until the native child exits: KernelSU
        // may emit late stdout/exit events after the JS timeout and expects the
        // window callback to still exist.
        fail(new ApiError("E_TIMEOUT", "命令执行超时", `${command.slice(0, 160)} (${timeoutMs}ms)`), true);
      }, timeoutMs);
    }
    (window as unknown as Record<string, SpawnChildProcess>)[callbackName] = child;
    try {
      spawnCommand("/system/bin/sh", JSON.stringify(["-c", shellQuote(command)]), "{}", callbackName);
    } catch (error) {
      fail(error);
    }
  });
}

export function parseEnvelope<T>(stdout: string, errno: number, stderr: string): T {
  const text = stdout.trim();
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end < start) {
    const detail = [stderr.trim(), text].filter(Boolean).join("\n").slice(0, 500);
    throw new ApiError("E_PROTOCOL", `命令没有返回 JSON（退出码 ${errno}）`, detail);
  }

  let parsed: ApiEnvelope<T>;
  try {
    parsed = JSON.parse(text.slice(start, end + 1)) as ApiEnvelope<T>;
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new ApiError("E_PROTOCOL", "JSON 解析失败", `${detail}\n${text.slice(0, 300)}`);
  }

  if (parsed.ok === true) return parsed.data;
  const payload = parsed.ok === false ? parsed.error : undefined;
  throw new ApiError(payload?.code || "E_UNKNOWN", payload?.message || "操作失败", payload?.detail || "");
}

export async function runJson<T>(args: string[], timeoutMs = 0): Promise<T> {
  const command = `${TOOL} ${args.map(shellQuote).join(" ")}`;
  const result = await execCommand(command, timeoutMs);
  return parseEnvelope<T>(result.stdout, result.errno, result.stderr);
}

function toBase64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

export async function runJsonWithFile<T>(args: string[], payload: unknown): Promise<T> {
  const path = `/data/local/tmp/box-ui-${Date.now()}-${tempCounter++}.json`;
  const encoded = toBase64(JSON.stringify(payload));
  await execCommand(`printf '%s' '${encoded}' | base64 -d > ${shellQuote(path)}`);
  try {
    return await runJson<T>([...args, path, "--json"]);
  } finally {
    try {
      await execCommand(`rm -f ${shellQuote(path)}`);
    } catch {
      // 临时文件清理失败不覆盖主操作结果
    }
  }
}

export const api = {
  status: () => runJson<StatusData>(["status", "--json"], 15_000),

  service: (action: ServiceAction) => runJson<{ action: string; output: string }>(["service", action, "--json"]),

  setControlAuto: () => runJson<{ mode: string; desired: string }>(["internal", "control-auto", "--json"]),
  setControlManual: () => runJson<{ mode: string; desired: string }>(["internal", "control-manual", "--json"]),

  subscriptions: async () => {
    const data = await runJson<{ items: Subscription[] }>(["sub", "list", "--json"]);
    return data.items;
  },

  subscription: (name: string) => runJson<SubscriptionDetail>(["sub", "get", name, "--json"]),

  addSubscription: (url: string, name: string, userAgent = "") =>
    runJson<Subscription>(["sub", "add", url, name, userAgent, "--json"]),

  setSubscription: (oldName: string, newName: string, url: string, userAgent?: string) =>
    runJson<Subscription>(
      userAgent === undefined
        ? ["sub", "set", oldName, newName, url, "--json"]
        : ["sub", "set", oldName, newName, url, userAgent, "--json"],
    ),

  removeSubscription: (name: string) => runJson<{ name: string }>(["sub", "rm", name, "--json"]),

  setSubscriptionEnabled: (name: string, enabled: boolean) =>
    runJson<{ name: string; enabled: boolean; reloaded: boolean; missing: boolean }>([
      "sub",
      enabled ? "enable" : "disable",
      name,
      "--json",
    ]),
  config: () => runJson<ConfigData>(["config", "get", "--json"]),

  ruleProfile: () => runJson<RuleProfileData>(["rule-profile", "get", "--json"]),

  ruleProfiles: () => runJson<RuleProfileListData>(["rule-profile", "list", "--json"]),

  addRuleProfile: (name: string, url: string) => runJson<RuleProfileItem>(["rule-profile", "add", name, url, "--json"]),

  editRuleProfile: (id: string, name: string, url: string) =>
    runJson<RuleProfileItem>(["rule-profile", "edit", id, name, url, "--json"]),

  updateRuleProfile: (id: string) => runJson<RuleProfileItem>(["rule-profile", "update", id, "--json"]),

  useRuleProfile: (id: string, reload: boolean) =>
    runJson<RuleProfileApplied>(["rule-profile", "use", id, ...(reload ? ["--reload"] : []), "--json"]),

  removeRuleProfile: (id: string) => runJson<{ removed: boolean; id: string }>(["rule-profile", "remove", id, "--json"]),

  deactivateRuleProfile: () => runJson<{ active: boolean }>(["rule-profile", "deactivate", "--json"]),

  setRuleProfile: (url: string) => runJson<RuleProfileData>(["rule-profile", "set", url, "--json"]),

  ruleProfileUrl: (id?: string) =>
    runJson<{ id?: string; url: string }>(id ? ["rule-profile", "url", id, "--json"] : ["rule-profile", "url", "--json"]),

  clearRuleProfile: () => runJson<{ cleared: boolean }>(["rule-profile", "clear", "--json"]),

  fetchRuleProfile: () => runJson<RuleProfileFetched>(["rule-profile", "fetch", "--json"]),

  applyRuleProfile: (reload: boolean) =>
    runJson<RuleProfileApplied>(reload ? ["rule-profile", "apply", "--reload", "--json"] : ["rule-profile", "apply", "--json"]),

  applyConfig: (config: ConfigData) => runJsonWithFile<{ changed: string[] }>(["config", "apply"], config),

  deleteConfig: () => runJson<{ path: string }>(["config", "delete", "--json"]),

  settings: () => runJson<SettingsData>(["settings", "get", "--json"]),

  applySettings: (settings: Record<string, string>, arrays: Record<string, string[]>, token?: string) => {
    const payload: Record<string, unknown> = { settings: { ...settings, ...arrays } };
    if (typeof token === "string") payload.githubtoken = token;
    return runJsonWithFile<{ changed: string[] }>(["settings", "apply"], payload);
  },

  logs: async (count = 200) => {
    const data = await runJson<LogsData>(["logs", String(count), "--json"]);
    return data;
  },

  clearLogs: () => runJson<{ cleared: string[] }>(["logs", "clear", "--json"]),

  resetSubscriptions: () => runJson<{ cleared: boolean }>(["reset", "subscriptions", "--json"]),

  taskStart: (kind: TaskKind, arg?: string) =>
    runJson<TaskInfo>(["task", "start", kind, ...(arg ? [arg] : []), "--json"]),
  taskLogs: async (id: string, count = 200) => {
    const data = await runJson<TaskLogsData>(["task", "logs", id, String(count), "--json"]);
    return data;
  },

  taskList: async () => {
    const data = await runJson<{ items: TaskInfo[] }>(["task", "list", "--json"]);
    return data.items;
  },

  taskCancel: (id: string) => runJson<TaskInfo>(["task", "cancel", id, "--json"]),

  coreConfig: () => runJson<CoreConfigData>(["core", "get", "--json"]),

  applyCoreConfig: (payload: { structured?: MihomoStructuredConfig; raw?: string }) =>
    runJsonWithFile<{ changed: string[]; mode: string }>(["core", "apply"], payload),

  appLists: () => runJson<AppListsData>(["lists", "get", "--json"]),

  applyAppLists: (payload: AppListsPayload) =>
    runJsonWithFile<{ changed: string[] }>(["lists", "apply"], payload),
};

type PackageBridgeMethod = (value: string) => unknown;

interface PackageBridgeMethods {
  listPackages: PackageBridgeMethod;
  getPackagesInfo: PackageBridgeMethod;
}

interface KsuPackageHost {
  pm?: PackageBridgeMethods | (() => PackageBridgeMethods | undefined);
  listPackages?: PackageBridgeMethod;
  getPackagesInfo?: PackageBridgeMethod;
}

export type PackageBridgeKind = "pm" | "legacy";

export interface ResolvedPackageBridge {
  kind: PackageBridgeKind;
  listPackages: PackageBridgeMethod;
  getPackagesInfo: PackageBridgeMethod;
}

export type PackageBridgeErrorCode =
  | "PACKAGE_BRIDGE_UNAVAILABLE"
  | "PACKAGE_BRIDGE_ERROR"
  | "PACKAGE_BRIDGE_BAD_RESPONSE";

export class PackageBridgeError extends Error {
  readonly code: PackageBridgeErrorCode;

  constructor(code: PackageBridgeErrorCode, message: string) {
    super(message);
    this.name = "PackageBridgeError";
    this.code = code;
  }
}

function getKsuPackageHost(): KsuPackageHost | undefined {
  return (window as Window & { ksu?: KsuPackageHost }).ksu;
}

function hasPackageMethods(value: unknown): value is PackageBridgeMethods {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<PackageBridgeMethods>;
  return typeof candidate.listPackages === "function" && typeof candidate.getPackagesInfo === "function";
}

export function resolvePackageBridge(): ResolvedPackageBridge | null {
  const ksu = getKsuPackageHost();
  if (!ksu) return null;

  try {
    const pmFunction = ksu.pm;
    const pm = typeof pmFunction === "function" ? pmFunction() : pmFunction;
    if (hasPackageMethods(pm)) {
      return {
        kind: "pm",
        listPackages: (type) => pm.listPackages(type),
        getPackagesInfo: (packages) => pm.getPackagesInfo(packages),
      };
    }
  } catch {
    // pm() may exist but be unavailable on older managers; fall through to legacy.
  }

  if (typeof ksu.listPackages === "function" && typeof ksu.getPackagesInfo === "function") {
    return {
      kind: "legacy",
      listPackages: (type) => ksu.listPackages?.(type),
      getPackagesInfo: (packages) => ksu.getPackagesInfo?.(packages),
    };
  }

  return null;
}

export function packageManagerAvailable(): boolean {
  return resolvePackageBridge() !== null;
}

function parseBridgeArray(value: unknown): unknown[] {
  let parsed: unknown = value;
  if (typeof value === "string") {
    try {
      parsed = JSON.parse(value) as unknown;
    } catch {
      throw new PackageBridgeError("PACKAGE_BRIDGE_BAD_RESPONSE", "应用列表返回了无法解析的数据");
    }
  }
  if (!Array.isArray(parsed)) {
    throw new PackageBridgeError("PACKAGE_BRIDGE_BAD_RESPONSE", "应用列表返回格式不正确");
  }
  return parsed;
}

export async function listInstalledPackages(): Promise<PackageInfo[]> {
  const resolved = resolvePackageBridge();
  if (!resolved) {
    throw new PackageBridgeError("PACKAGE_BRIDGE_UNAVAILABLE", "当前管理器未提供包列表接口");
  }

  try {
    const namesRaw = parseBridgeArray(resolved.listPackages("user"));
    const names = [...new Set(namesRaw.filter((value): value is string => typeof value === "string" && value.length > 0))];
    if (names.length === 0) return [];

    const raw = parseBridgeArray(resolved.getPackagesInfo(JSON.stringify(names)));
    const unique = new Map<string, PackageInfo>();
    for (const value of raw) {
      if (!value || typeof value !== "object") continue;
      const item = value as Record<string, unknown>;
      const packageName = String(item.packageName || "");
      if (!packageName || item.error) continue;
      if (unique.has(packageName)) continue;
      unique.set(packageName, {
        packageName,
        appLabel: String(item.appLabel || packageName),
        versionName: String(item.versionName || ""),
        isSystem: typeof item.isSystem === "boolean" ? item.isSystem : null,
        uid: typeof item.uid === "number" ? item.uid : null,
      });
    }
    return [...unique.values()].sort((a, b) => a.appLabel.localeCompare(b.appLabel, "zh-CN"));
  } catch (error) {
    if (error instanceof PackageBridgeError) throw error;
    throw new PackageBridgeError("PACKAGE_BRIDGE_ERROR", "读取应用列表失败");
  }
}
