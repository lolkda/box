import { exec } from "kernelsu";
import type {
  ApiEnvelope,
  AppListsData,
  AppListsPayload,
  ConfigData,
  CoreConfigData,
  MihomoStructuredConfig,
  LogsData,
  PackageInfo,
  ServiceAction,
  SettingsData,
  StatusData,
  Subscription,
  SubscriptionDetail,
  TaskInfo,
  TaskKind,
  TaskLogsData,
} from "./types";

export const TOOL = "/data/adb/box/scripts/box.tool";

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

export function isKernelsuAvailable(): boolean {
  return typeof window !== "undefined" && typeof (window as Window & { ksu?: unknown }).ksu !== "undefined";
}

async function execRaw(command: string): Promise<{ errno: number; stdout: string; stderr: string }> {
  if (!isKernelsuAvailable()) {
    throw new ApiError("E_UNAVAILABLE", "KernelSU API 不可用，请从 KernelSU 模块页打开");
  }
  try {
    return await exec(command);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new ApiError("E_EXEC", "执行命令失败", message);
  }
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

export async function runJson<T>(args: string[]): Promise<T> {
  const command = `${TOOL} ${args.map(shellQuote).join(" ")}`;
  const result = await execRaw(command);
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
  await execRaw(`printf '%s' '${encoded}' | base64 -d > ${shellQuote(path)}`);
  try {
    return await runJson<T>([...args, path, "--json"]);
  } finally {
    try {
      await execRaw(`rm -f ${shellQuote(path)}`);
    } catch {
      // 临时文件清理失败不覆盖主操作结果
    }
  }
}

export const api = {
  status: () => runJson<StatusData>(["status", "--json"]),

  service: (action: ServiceAction) => runJson<{ action: string; output: string }>(["service", action, "--json"]),

  subscriptions: async () => {
    const data = await runJson<{ items: Subscription[] }>(["sub", "list", "--json"]);
    return data.items;
  },

  subscription: (name: string) => runJson<SubscriptionDetail>(["sub", "get", name, "--json"]),

  addSubscription: (url: string, name: string) =>
    runJson<Subscription>(["sub", "add", url, name, "--json"]),

  setSubscription: (oldName: string, newName: string, url: string) =>
    runJson<Subscription>(["sub", "set", oldName, newName, url, "--json"]),

  removeSubscription: (name: string) => runJson<{ name: string }>(["sub", "rm", name, "--json"]),
  config: () => runJson<ConfigData>(["config", "get", "--json"]),

  applyConfig: (config: ConfigData) => runJsonWithFile<{ changed: string[] }>(["config", "apply"], config),

  deleteConfig: () => runJson<{ path: string }>(["config", "delete", "--json"]),

  settings: () => runJson<SettingsData>(["settings", "get", "--json"]),

  applySettings: (settings: Record<string, string>, arrays: Record<string, string[]>, token?: string) => {
    const payload: Record<string, unknown> = { ...settings, ...arrays };
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

interface KsuPackageBridge {
  listPackages(type: string): string;
  getPackagesInfo(packages: string): string;
}

export function packageManagerAvailable(): boolean {
  const ksu = (window as Window & { ksu?: { pm?: () => KsuPackageBridge } }).ksu;
  return typeof ksu?.pm === "function";
}

export async function listInstalledPackages(): Promise<PackageInfo[]> {
  const ksu = (window as Window & { ksu?: { pm?: () => KsuPackageBridge } }).ksu;
  if (typeof ksu?.pm !== "function") return [];
  try {
    const bridge = ksu.pm();
    const names = JSON.parse(bridge.listPackages("user")) as string[];
    if (!Array.isArray(names) || names.length === 0) return [];
    const raw = JSON.parse(bridge.getPackagesInfo(JSON.stringify(names))) as Array<Record<string, unknown>>;
    const unique = new Map<string, PackageInfo>();
    for (const item of raw) {
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
  } catch {
    return [];
  }
}
