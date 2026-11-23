import type { ConfigData, SettingsData, SubscriptionDetail, TaskInfo } from "../api/types";

interface MockState {
  running: boolean;
  pid: number | null;
  core: string;
  mode: string;
  subscriptions: SubscriptionDetail[];
  config: ConfigData;
  configExists: boolean;
  settings: Record<string, string>;
  arrays: Record<string, string[]>;
  tokenSet: boolean;
  tokenPreview: string;
  logs: string[];
  tasks: TaskInfo[];
  ruleProfileUrl: string;
  ruleProfileUpdatedAt: string;
}

const now = () => new Date().toISOString();

function buildSettings(): Pick<MockState, "settings" | "arrays" | "tokenSet" | "tokenPreview"> {
  return {
    settings: {
      auto_modify_config: "true",
      boot_auto_start: "true",
      bin_name: "mihomo",
      network_mode: "tproxy",
      tproxy_port: "9898",
      redir_port: "9797",
      ipv6: "true",
      performance_mode: "false",
      quic: "enable",
      mihomo_dns_forward: "enable",
      proxy_tcp: "true",
      proxy_udp: "true",
      dns_hijack_tcp: "true",
      dns_hijack_udp: "true",
      dns_hijack_mode: "tproxy",
      bypass_cn_ip: "false",
      bypass_cn_ip_v4: "false",
      bypass_cn_ip_v6: "false",
      cnip_mode: "ipset",
      proxy_mode: "core",
      update_subscription: "true",
      update_geo: "false",
      use_ghproxy: "true",
      url_ghproxy: "https://ghfast.top",
      mihomo_stable: "enable",
      singbox_stable: "enable",
      run_crontab: "false",
      interva_update: "0 0,6,12,18 * * *",
      cgroup_memcg: "false",
      memcg_limit: "100M",
      taskset_cpu: "false",
      allow_cpu: "0-7",
      cgroup_blkio: "false",
      weight: "",
      mac_filter: "false",
      mac_mode: "blacklist",
      ruleset_prefix: "",
      subscription_keep_last_good: "true",
      enable_network_service_control: "false",
      use_module_on_wifi: "true",
      use_module_on_wifi_disconnect: "true",
      use_ssid_matching: "false",
      use_wifi_list_mode: "blacklist",
      inotify_log_enabled: "true",
      subscription_mode: "auto",
    },
    arrays: {
      cnip_force_proxy_apps: [],
      force_proxy_cidrs: [],
      force_proxy_cidrs6: [],
      macs_list: [],
      wifi_ssids_list: ["Home 5G"],
      wifi_bssids_list: [],
    },
    tokenSet: true,
    tokenPreview: "ghp_****9xQ",
  };
}

const state: MockState = {
  running: true,
  pid: 23456,
  core: "mihomo",
  mode: "tproxy",
  subscriptions: [
    {
      name: "机场 A",
      url: "https://example.com/api/v1/client/subscribe?token=abcdef123456",
      url_masked: "https://example.com/***",
      provider: "downloaded",
      provider_size: 184320,
      provider_count: 0,
      updated_at: "2026-11-23T07:42:31+0800",
    },
    {
      name: "备用",
      url: "https://example.org/sub?token=xyz",
      url_masked: "https://example.org/***",
      provider: "missing",
      provider_size: 0,
      provider_count: 0,
      updated_at: "",
    },
    {
      name: "测试",
      url: "https://very-long-subscription-provider-domain.example.net/subscribe/abcdefghijklmnopqrstuvwxyz",
      url_masked: "https://very-long-subscription-provider-domain.example.net/***",
      provider: "downloaded",
      provider_size: 92160,
      provider_count: 0,
      updated_at: "2026-11-23T07:43:10+0800",
    },
  ],
  config: {
    "proxy-groups": [
      { name: "国外代理", type: "select", "include-all": true, "exclude-type": "dns", icon: "https://example.com/proxy.png" },
      { name: "国内直连", type: "select", proxies: ["DIRECT"], "include-all": true, "exclude-type": "dns" },
      { name: "漏网之鱼", type: "select", proxies: ["国外代理", "国内直连"] },
    ],
    "rule-providers": {
      "Fake-IP-Filter": {
        type: "http",
        behavior: "domain",
        format: "mrs",
        interval: 86400,
        path: "./rule_provider/Fake-IP-Filter.mrs",
        url: "https://raw.githubusercontent.com/DustinWin/ruleset_geodata/refs/heads/mihomo-ruleset/fakeip-filter.mrs",
      },
      proxy_domain: {
        type: "http",
        behavior: "domain",
        format: "mrs",
        interval: 86400,
        path: "./rule_provider/proxy_domain.mrs",
        url: "https://raw.githubusercontent.com/DustinWin/ruleset_geodata/refs/heads/mihomo-ruleset/proxy.mrs",
      },
      cn_ip: {
        type: "http",
        behavior: "ipcidr",
        format: "mrs",
        interval: 86400,
        path: "./rule_provider/cn_ip.mrs",
        url: "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/refs/heads/meta/geo/geoip/cn.mrs",
      },
      private_domain: {
        type: "http",
        behavior: "domain",
        format: "mrs",
        interval: 86400,
        path: "./rule_provider/private_domain.mrs",
        url: "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/refs/heads/meta/geo/geosite/private.mrs",
      },
    },
    rules: [
      "DST-PORT,53,DNS_Hijack",
      "RULE-SET,proxy_domain,国外代理",
      "OR,((RULE-SET,cn_ip),(RULE-SET,cn_ip),(RULE-SET,private_domain)),国内直连",
      "MATCH,漏网之鱼",
    ],
  },
  configExists: true,
  ruleProfileUrl: "",
  ruleProfileUpdatedAt: "",
  ...buildSettings(),
  logs: [
    "2026-11-23 07:40:12 [Info]: Box 服务已启动",
    "2026-11-23 07:40:12 [Info]: 核心: mihomo, 网络模式: tproxy",
    "2026-11-23 07:41:02 [Warn]: 规则集更新较慢，已保留旧版本",
    "2026-11-23 07:42:31 [Info]: 订阅 机场 A 更新成功，节点数 42",
    "2026-11-23 07:43:10 [Error]: 订阅 备用 下载失败: HTTP 502",
    "2026-11-23 07:43:10 [Info]: 已保留上一个可用节点文件",
    "2026-11-23 07:44:05 [Info]: 配置重载成功",
    "2026-11-23 07:45:18 [Debug]: DNS 缓存清理完成",
  ],
  tasks: [
    {
      id: "20261123073000-2201",
      kind: "sub-update",
      title: "更新订阅",
      state: "success",
      pid: 2201,
      started_at: "2026-11-23T07:30:00+0800",
      finished_at: "2026-11-23T07:30:42+0800",
      exit_code: 0,
      cancelable: true,
      output_size: 420,
    },
  ],
};

function envelope<T>(data: T) {
  return { ok: true, data, meta: { version: 1 } };
}

function failure(code: string, message: string, detail = "") {
  return { ok: false, error: { code, message, detail }, meta: { version: 1 } };
}

function parseTokens(command: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let inSingle = false;
  for (let i = 0; i < command.length; i += 1) {
    const char = command[i];
    if (char === "'" && !inSingle) {
      inSingle = true;
      continue;
    }
    if (char === "'" && inSingle) {
      inSingle = false;
      continue;
    }
    if (char === "\\" && !inSingle) {
      i += 1;
      current += command[i] || "";
      continue;
    }
    if (!inSingle && /\s/.test(char)) {
      if (current) {
        tokens.push(current);
        current = "";
      }
      continue;
    }
    current += char;
  }
  if (current) tokens.push(current);
  return tokens;
}

function getArgs(command: string): string[] {
  const tokens = parseTokens(command);
  const index = tokens.findIndex((token) => token.endsWith("box.tool"));
  return index >= 0 ? tokens.slice(index + 1) : [];
}

const tempFiles = new Map<string, string>();

function fileDataFromCommand(command: string): { path: string; content: string } | null {
  const match = command.match(/^printf '%s' '([^']*)' \| base64 -d > '([^']+)'$/);
  if (!match) return null;
  try {
    const binary = atob(match[1]);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return { path: match[2], content: new TextDecoder().decode(bytes) };
  } catch {
    return null;
  }
}

function taskOf(id: string): TaskInfo | undefined {
  return state.tasks.find((task) => task.id === id);
}

function startTask(kind: string, arg?: string): TaskInfo {
  const cancelable = kind !== "upkernel";
  const id = `20261123${String(Date.now()).slice(-6)}-${Math.floor(Math.random() * 900 + 100)}`;
  const title = kind === "upkernel" ? `更新 ${arg || "核心"}` : kind === "sub-update" ? "更新订阅" : "维护任务";
  const task: TaskInfo = {
    id,
    kind,
    title,
    state: "running",
    pid: Math.floor(Math.random() * 9000 + 1000),
    started_at: now(),
    finished_at: "",
    exit_code: null,
    cancelable,
    output_size: 0,
  };
  state.tasks.unshift(task);
  const output = [
    `[mock] ${title} 已启动`,
    kind === "sub-update" ? "[mock] 开始下载订阅节点…" : "[mock] 开始下载核心文件…",
    kind === "upkernel" ? "[mock] 校验架构与文件完整性…" : "[mock] 校验返回内容…",
  ];
  let index = 0;
  const timer = window.setInterval(() => {
    if (task.state !== "running") {
      window.clearInterval(timer);
      return;
    }
    if (index < output.length) {
      state.logs.push(`2026-11-23 08:20:0${index} [Info]: ${output[index]}`);
      task.output_size += output[index].length + 30;
      index += 1;
    }
  }, 350);
  window.setTimeout(() => {
    window.clearInterval(timer);
    if (task.state !== "running") return;
    task.state = "success";
    task.exit_code = 0;
    task.finished_at = now();
    state.logs.push(`2026-11-23 08:20:12 [Info]: ${title}完成`);
  }, kind === "upkernel" ? 5500 : 2600);
  state.logs.push(`2026-11-23 08:20:00 [Info]: ${title} 已启动`);
  return task;
}

function handle(args: string[]): unknown {
  const [resource, action, ...rest] = args;
  const json = args.includes("--json");
  if (!json) return failure("E_MOCK", "mock 只支持 --json 调用");

  switch (resource) {
    case "status":
      return envelope({
        running: state.running,
        pid: state.pid,
        core: state.core,
        mode: state.mode,
        config: "config.yaml",
        config_exists: state.configExists,
        subscriptions: state.subscriptions.length,
        active_tasks: state.tasks.filter((task) => task.state === "running").length,
      });
    case "service": {
      if (action === "start" || action === "restart") {
        state.running = true;
        state.pid = Math.floor(Math.random() * 9000 + 1000);
        state.logs.push(`2026-11-23 08:21:00 [Info]: 服务${action === "start" ? "启动" : "重启"}成功`);
      } else if (action === "stop") {
        state.running = false;
        state.pid = null;
        state.logs.push("2026-11-23 08:21:00 [Info]: 服务已停止");
      } else {
        state.logs.push("2026-11-23 08:21:00 [Info]: 配置重载成功");
      }
      return envelope({ action, output: "" });
    }
    case "sub": {
      if (action === "list") return envelope({ items: state.subscriptions.map(({ url, ...item }) => ({ ...item, url_masked: item.url_masked })) });
      if (action === "get") {
        const found = state.subscriptions.find((item) => item.name === rest[0]);
        return found ? envelope(found) : failure("E_NOT_FOUND", `订阅不存在: ${rest[0]}`);
      }
      if (action === "add") {
        const url = rest[0] || "";
        const name = rest[1] || new URL(url).hostname;
        if (state.subscriptions.some((item) => item.name === name)) return failure("E_CONFLICT", `订阅已存在: ${name}`);
        state.subscriptions.push({ name, url, url_masked: url.replace(/\/[^/]*$/, "/***"), provider: "missing", provider_size: 0, provider_count: 0, updated_at: "" });
        return envelope({ name, url_masked: url.replace(/\/[^/]*$/, "/***") });
      }
      if (action === "set") {
        const [oldName, newName, url] = rest;
        const item = state.subscriptions.find((entry) => entry.name === oldName);
        if (!item) return failure("E_NOT_FOUND", `订阅不存在: ${oldName}`);
        item.name = newName || oldName;
        item.url = url;
        item.url_masked = url.replace(/\/[^/]*$/, "/***");
        return envelope({ name: item.name, url_masked: item.url_masked });
      }
      if (action === "rm") {
        state.subscriptions = state.subscriptions.filter((item) => item.name !== rest[0]);
        return envelope({ name: rest[0] });
      }
      if (action === "update") return envelope({ updated: true, partial: false, failed: false });
      break;
    }
    case "rule-profile": {
      const counts = {
        groups: state.config["proxy-groups"].length,
        rule_providers: Object.keys(state.config["rule-providers"]).length,
        rules: state.config.rules.length,
      };
      const profile = {
        supported: state.core === "mihomo",
        configured: Boolean(state.ruleProfileUrl),
        url_masked: state.ruleProfileUrl ? state.ruleProfileUrl.replace(/\/[^/]*$/, "/***") : "",
        updated_at: state.ruleProfileUpdatedAt,
        counts,
      };
      if (action === "get") return envelope(profile);
      if (action === "url") return state.ruleProfileUrl ? envelope({ url: state.ruleProfileUrl }) : failure("E_NOT_FOUND", "还没有配置规则与策略订阅");
      if (action === "set") {
        const url = rest[0] || "";
        if (!/^https?:\/\//.test(url)) return failure("E_VALIDATION", "规则模板地址必须以 http:// 或 https:// 开头");
        state.ruleProfileUrl = url;
        return envelope({ ...profile, configured: true, url_masked: url.replace(/\/[^/]*$/, "/***") });
      }
      if (action === "clear") {
        state.ruleProfileUrl = "";
        state.ruleProfileUpdatedAt = "";
        return envelope({ cleared: true });
      }
      if (action === "fetch") {
        return envelope({
          url_masked: profile.url_masked,
          fetched_at: now(),
          counts: { groups: 22, rule_providers: 33, rules: 49 },
        });
      }
      if (action === "apply") {
        state.ruleProfileUpdatedAt = now();
        return envelope({
          changed: ["proxy-groups", "rule-providers", "rules"],
          reloaded: rest.includes("--reload") ? state.running : false,
          updated_at: state.ruleProfileUpdatedAt,
          counts,
        });
      }
      break;
    }
    case "config": {
      if (action === "get") {
        if (!state.configExists) return failure("E_NOT_FOUND", "配置文件不存在");
        return envelope(state.config);
      }
      if (action === "apply") {
        const path = rest[0];
        const content = path ? tempFiles.get(path) : undefined;
        if (!content) return failure("E_IO", "未找到临时配置文件");
        try {
          const parsed = JSON.parse(content) as ConfigData;
          state.config = parsed;
          state.configExists = true;
          return envelope({ changed: ["proxy-groups", "rule-providers", "rules"] });
        } catch {
          return failure("E_VALIDATION", "配置 JSON 无法解析");
        }
      }
      if (action === "delete") {
        state.configExists = false;
        return envelope({ path: "/data/adb/box/mihomo/config.yaml.deleted" });
      }
      break;
    }
    case "settings": {
      if (action === "get") {
        return envelope({
          ...state.settings,
          ...state.arrays,
          sensitive: { githubtoken: { set: state.tokenSet, preview: state.tokenPreview } },
        } satisfies SettingsData);
      }
      if (action === "apply") {
        const path = rest[0];
        const content = path ? tempFiles.get(path) : undefined;
        if (!content) return failure("E_IO", "未找到临时设置文件");
        try {
          const payload = JSON.parse(content) as Record<string, unknown>;
          for (const [key, value] of Object.entries(payload)) {
            if (key === "githubtoken") {
              state.tokenSet = Boolean(value);
              state.tokenPreview = value ? `${String(value).slice(0, 4)}****${String(value).slice(-4)}` : "";
            } else if (Array.isArray(value)) {
              state.arrays[key] = value.map(String);
            } else {
              state.settings[key] = String(value);
            }
          }
          return envelope({ changed: Object.keys(payload) });
        } catch {
          return failure("E_VALIDATION", "设置 JSON 无法解析");
        }
      }
      break;
    }
    case "logs": {
      if (action === "clear") {
        state.logs = [];
        return envelope({ cleared: ["/data/adb/box/run/runs.log"] });
      }
      const count = Number(action) || 200;
      return envelope({ file: "/data/adb/box/run/runs.log", lines: state.logs.slice(-count) });
    }
    case "reset": {
      state.subscriptions = [];
      return envelope({ cleared: true });
    }
    case "task": {
      if (action === "start") {
        if (state.tasks.some((task) => task.state === "running")) return failure("E_BUSY", "已有任务正在运行");
        return envelope(startTask(rest[0], rest[1]));
      }
      if (action === "list") return envelope({ items: state.tasks });
      if (action === "status") {
        const task = taskOf(rest[0]);
        return task ? envelope(task) : failure("E_NOT_FOUND", "任务不存在");
      }
      if (action === "logs") {
        const task = taskOf(rest[0]);
        if (!task) return failure("E_NOT_FOUND", "任务不存在");
        return envelope({ id: task.id, file: `/data/adb/box/run/tasks/${task.id}/output.log`, lines: state.logs.slice(-Number(rest[1]) || -200) });
      }
      if (action === "cancel") {
        const task = taskOf(rest[0]);
        if (!task) return failure("E_NOT_FOUND", "任务不存在");
        if (!task.cancelable) return failure("E_VALIDATION", "该任务不可取消");
        task.state = "canceled";
        task.finished_at = now();
        return envelope(task);
      }
      break;
    }
  }
  return failure("E_MOCK", `mock 未实现: ${args.join(" ")}`);
}

function unwrapShellCommand(value: string): string {
  let command = value;
  if (command.startsWith("'") && command.endsWith("'")) command = command.slice(1, -1);
  return command.replace(/'\\''/g, "'");
}

function runMockCommand(command: string): { errno: number; stdout: string; stderr: string } {
  const fileData = fileDataFromCommand(command);
  if (fileData) {
    tempFiles.set(fileData.path, fileData.content);
    return { errno: 0, stdout: "", stderr: "" };
  }
  if (command.startsWith("rm -f ")) {
    const path = parseTokens(command)[2];
    if (path) tempFiles.delete(path);
    return { errno: 0, stdout: "", stderr: "" };
  }
  const args = getArgs(command);
  const result = handle(args) as { ok: boolean };
  return { errno: result.ok ? 0 : 1, stdout: JSON.stringify(result), stderr: "" };
}

export function installMockKsu() {
  const target = window as Window & {
    ksu?: Record<string, unknown>;
  };
  target.ksu = {
    exec(command: string, _options: string, callbackName: string) {
      const result = runMockCommand(command);
      window.setTimeout(() => {
        (window as unknown as Record<string, (errno: number, stdout: string, stderr: string) => void>)[callbackName]?.(
          result.errno,
          result.stdout,
          result.stderr,
        );
      }, 30);
    },
    spawn(_command: string, args: string, _options: string, callbackName: string) {
      const shellArgs = JSON.parse(args) as string[];
      const result = runMockCommand(unwrapShellCommand(shellArgs[1] || ""));
      window.setTimeout(() => {
        const child = (window as unknown as Record<string, {
          stdout: { emit: (event: string, data: string) => void };
          stderr: { emit: (event: string, data: string) => void };
          emit: (event: string, ...payload: unknown[]) => void;
        }>)[callbackName];
        if (!child) return;
        if (result.stdout) for (const line of result.stdout.split("\n")) child.stdout.emit("data", line);
        if (result.stderr) for (const line of result.stderr.split("\n")) child.stderr.emit("data", line);
        child.emit("exit", result.errno);
      }, 30);
    },
    toast() {},
    fullScreen() {},
    enableEdgeToEdge() {},
    moduleInfo() {
      return "{}";
    },
    pm() {
      const packages = ["com.android.settings", "com.android.shell", "com.example.proxy", "com.example.video"];
      return {
        version: () => 2,
        listPackages: () => JSON.stringify(packages),
        getPackagesInfo: () => JSON.stringify(packages.map((packageName, index) => ({
          packageName,
          appLabel: ["设置", "Shell", "Proxy 工具", "视频示例"][index],
          versionName: "1.0",
          versionCode: 1,
          isSystem: index < 2,
          uid: 10000 + index,
        }))),
      };
    },
  };
}
