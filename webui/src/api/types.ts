export type ApiEnvelope<T> =
  | { ok: true; data: T; meta?: { version?: number } }
  | { ok: false; error: ApiErrorPayload; meta?: { version?: number } };

export interface ApiErrorPayload {
  code: string;
  message: string;
  detail?: string;
}

export interface StatusData {
  running: boolean;
  pid: number | null;
  core: string;
  mode: string;
  config: string;
  config_exists: boolean;
  subscriptions: number;
  active_tasks: number;
}

export interface Subscription {
  name: string;
  url_masked: string;
  provider: "downloaded" | "missing" | string;
  provider_size: number;
  updated_at: string;
}

export interface SubscriptionDetail extends Subscription {
  url: string;
}

export interface ProxyGroup {
  name: string;
  type: string;
  proxies?: string[];
  "include-all"?: boolean;
  "exclude-type"?: string;
  icon?: string;
  [key: string]: unknown;
}

export interface RuleProvider {
  type: string;
  behavior: string;
  format: string;
  interval: number;
  path?: string;
  url?: string;
  proxy?: string;
  payload?: string[];
  [key: string]: unknown;
}

export interface ConfigData {
  "proxy-groups": ProxyGroup[];
  "rule-providers": Record<string, RuleProvider>;
  rules: string[];
}

export interface SettingsData {
  settings: Record<string, string>;
  arrays: Record<string, string[]>;
  sensitive: {
    githubtoken: {
      set: boolean;
      preview: string;
    };
  };
}

export interface LogsData {
  file: string;
  lines: string[];
}

export type TaskState = "running" | "success" | "failed" | "canceled" | "interrupted" | "unknown";

export interface TaskInfo {
  id: string;
  kind: string;
  title: string;
  state: TaskState;
  pid: number | null;
  started_at: string;
  finished_at: string;
  exit_code: number | null;
  cancelable: boolean;
  output_size: number;
}

export interface TaskLogsData {
  id: string;
  file: string;
  lines: string[];
}

export type ServiceAction = "start" | "stop" | "restart" | "reload";

export type TaskKind = "sub-update" | "upkernel" | "upcnip" | "geox" | "xui";

export interface PackageInfo {
  packageName: string;
  appLabel: string;
  versionName: string;
  isSystem: boolean | null;
  uid: number | null;
}
