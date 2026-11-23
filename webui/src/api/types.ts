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
  provider: "downloaded" | "missing" | "aggregated" | string;
  provider_size: number;
  provider_count: number;
  updated_at: string;
  enabled: boolean;
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

export interface SettingsSensitive {
  githubtoken: {
    set: boolean;
    preview: string;
  };
}

export interface SettingsData {
  [key: string]: string | string[] | SettingsSensitive;
  sensitive: SettingsSensitive;
}

export interface LogsData {
  file: string;
  lines: string[];
}
export interface RuleProfileCounts {
  groups: number;
  rule_providers: number;
  rules: number;
}

export interface RuleProfileData {
  supported: boolean;
  configured: boolean;
  id?: string;
  name?: string;
  url_masked: string;
  updated_at: string;
  counts: RuleProfileCounts;
}

export interface RuleProfileItem {
  id: string;
  name: string;
  url_masked: string;
  active: boolean;
  pending: boolean;
  fetched_at: string;
  applied_at: string;
  counts: RuleProfileCounts;
  can_delete: boolean;
}

export interface RuleProfileListData {
  supported: boolean;
  items: RuleProfileItem[];
}

export interface RuleProfileFetched {
  url_masked: string;
  fetched_at: string;
  counts: RuleProfileCounts;
}

export interface RuleProfileApplied {
  id?: string;
  name?: string;
  changed: string[];
  reloaded: boolean;
  updated_at: string;
  counts: RuleProfileCounts;
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

export interface MihomoStructuredConfig {
  mode: string;
  "log-level": string;
  "allow-lan": boolean;
  "external-controller": string;
  "external-ui": string;
  "profile.store-selected": boolean;
  "profile.store-fake-ip": boolean;
  "unified-delay": boolean;
  "tcp-concurrent": boolean;
  "find-process-mode": string;
}

export interface CoreConfigData {
  core: string;
  file: string;
  structured: MihomoStructuredConfig;
  raw: string;
}

export interface PackageEntry {
  uid?: string;
  package: string;
}

export interface AppListsData {
  package: PackageEntry[];
  gid: string[];
  interfaces: {
    allow: string[];
    ignore: string[];
  };
  files: {
    package: string;
    gid: string;
    interfaces: string;
  };
}

export interface AppListsPayload {
  package: PackageEntry[];
  gid: string[];
  interfaces: {
    allow: string[];
    ignore: string[];
  };
}
