import { useEffect, useMemo, useState } from "preact/hooks";
import { exec } from "kernelsu";

const TOOL = "/data/adb/box/scripts/box.tool";
const TMP = "/data/local/tmp/box-ui.json";

async function run(command) {
  if (typeof exec !== "function") {
    throw new Error("KernelSU API 不可用，请从 KernelSU 模块页打开");
  }
  const result = await exec(command);
  if (result.errno !== 0) {
    throw new Error((result.stderr || result.stdout || `退出码 ${result.errno}`).trim());
  }
  return result.stdout || "";
}

async function runJson(command) {
  const text = await run(command);
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end < 0) throw new Error(`返回内容不是 JSON: ${text.slice(0, 120)}`);
  return JSON.parse(text.slice(start, end + 1));
}

function toBase64(text) {
  return btoa(unescape(encodeURIComponent(text)));
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

async function writeTemp(value) {
  await run(`printf '%s' '${toBase64(JSON.stringify(value))}' | base64 -d > ${TMP}`);
  return TMP;
}

function Card({ title, actions, children }) {
  return (
    <section class="card">
      <header class="card-head">
        <h2>{title}</h2>
        <div class="actions">{actions}</div>
      </header>
      {children}
    </section>
  );
}

function Field({ label, children }) {
  return (
    <label class="field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function StatusTab({ status, busy, onAction }) {
  return (
    <Card
      title="运行状态"
      actions={
        <>
          <button disabled={busy} onClick={() => onAction("start")}>启动</button>
          <button disabled={busy} onClick={() => onAction("stop")}>停止</button>
          <button disabled={busy} onClick={() => onAction("restart")}>重启</button>
          <button disabled={busy} onClick={() => onAction("reload")}>重载</button>
        </>
      }
    >
      <div class="grid">
        <div class="stat"><span>状态</span><strong>{status?.running ? "运行中" : "已停止"}</strong></div>
        <div class="stat"><span>PID</span><strong>{status?.pid ?? "-"}</strong></div>
        <div class="stat"><span>核心</span><strong>{status?.core ?? "-"}</strong></div>
        <div class="stat"><span>模式</span><strong>{status?.mode ?? "-"}</strong></div>
        <div class="stat"><span>配置</span><strong>{status?.config ?? "-"}</strong></div>
        <div class="stat"><span>订阅</span><strong>{status?.subscriptions ?? 0}</strong></div>
      </div>
      <a class="link" href="http://127.0.0.1:9090/ui/" target="_blank" rel="noreferrer">打开面板</a>
    </Card>
  );
}

function SubsTab({ items, busy, onRefresh, onAdd, onRemove, onUpdate }) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");

  return (
    <Card
      title="订阅"
      actions={<button disabled={busy} onClick={onRefresh}>刷新</button>}
    >
      <div class="form-row">
        <input placeholder="名字" value={name} onInput={(e) => setName(e.currentTarget.value)} />
        <input placeholder="https://..." value={url} onInput={(e) => setUrl(e.currentTarget.value)} />
        <button
          disabled={busy || !url.trim()}
          onClick={async () => {
            await onAdd(name.trim(), url.trim());
            setName("");
            setUrl("");
          }}
        >
          添加
        </button>
        <button disabled={busy} onClick={onUpdate}>更新全部</button>
      </div>
      {items.length === 0 ? <p class="muted">暂无订阅</p> : (
        <table>
          <thead><tr><th>名字</th><th>地址</th><th>节点文件</th><th /></tr></thead>
          <tbody>
            {items.map((item) => (
              <tr key={item.name}>
                <td>{item.name}</td>
                <td class="mono ellipsis">{item.url}</td>
                <td>{item.provider}</td>
                <td><button class="danger" disabled={busy} onClick={() => onRemove(item.name)}>删除</button></td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}

function GroupsTab({ config, busy, onSave }) {
  const [groups, setGroups] = useState(config["proxy-groups"] || []);
  useEffect(() => setGroups(config["proxy-groups"] || []), [config]);

  const update = (index, patch) => {
    setGroups((current) => current.map((item, i) => (i === index ? { ...item, ...patch } : item)));
  };

  return (
    <Card
      title="策略组"
      actions={
        <>
          <button onClick={() => setGroups((c) => [...c, { name: "新策略", type: "select", proxies: [], "include-all": true }])}>新增</button>
          <button disabled={busy} onClick={() => onSave({ ...config, "proxy-groups": groups })}>保存</button>
        </>
      }
    >
      {groups.map((group, index) => (
        <div class="sub-card" key={index}>
          <div class="form-row">
            <input value={group.name || ""} onInput={(e) => update(index, { name: e.currentTarget.value })} />
            <select value={group.type || "select"} onChange={(e) => update(index, { type: e.currentTarget.value })}>
              <option value="select">select</option>
              <option value="url-test">url-test</option>
              <option value="fallback">fallback</option>
            </select>
            <label class="check">
              <input type="checkbox" checked={group["include-all"] !== false} onChange={(e) => update(index, { "include-all": e.currentTarget.checked })} />
              include-all
            </label>
            <button class="danger" onClick={() => setGroups((c) => c.filter((_, i) => i !== index))}>删除</button>
          </div>
          <input
            placeholder="成员，逗号分隔"
            value={(group.proxies || []).join(", ")}
            onInput={(e) => update(index, { proxies: e.currentTarget.value.split(",").map((v) => v.trim()).filter(Boolean) })}
          />
        </div>
      ))}
    </Card>
  );
}

function RulesTab({ config, busy, onSave }) {
  const [rules, setRules] = useState(config.rules || []);
  useEffect(() => setRules(config.rules || []), [config]);

  return (
    <Card
      title="规则"
      actions={
        <>
          <button onClick={() => setRules((c) => [...c, "DOMAIN-SUFFIX,example.com,DIRECT"])}>新增</button>
          <button disabled={busy} onClick={() => onSave({ ...config, rules })}>保存</button>
        </>
      }
    >
      {rules.map((rule, index) => (
        <div class="form-row" key={index}>
          <input class="mono" value={rule} onInput={(e) => setRules((c) => c.map((v, i) => (i === index ? e.currentTarget.value : v)))} />
          <button class="danger" onClick={() => setRules((c) => c.filter((_, i) => i !== index))}>删除</button>
        </div>
      ))}
    </Card>
  );
}

function ProvidersTab({ config, busy, onSave }) {
  const [providers, setProviders] = useState(config["rule-providers"] || {});
  useEffect(() => setProviders(config["rule-providers"] || {}), [config]);

  const entries = Object.entries(providers);

  const update = (name, patch) => {
    setProviders((current) => ({ ...current, [name]: { ...current[name], ...patch } }));
  };
  const rename = (oldName, newName) => {
    setProviders((current) => {
      const next = {};
      for (const [key, value] of Object.entries(current)) next[key === oldName ? newName : key] = value;
      return next;
    });
  };

  return (
    <Card
      title="规则集"
      actions={
        <>
          <button onClick={() => setProviders((c) => ({ ...c, [`规则集${Object.keys(c).length + 1}`]: { type: "http", behavior: "domain", format: "yaml", interval: 86400, path: "./ruleset/new.yaml", url: "" } }))}>新增</button>
          <button disabled={busy} onClick={() => onSave({ ...config, "rule-providers": providers })}>保存</button>
        </>
      }
    >
      {entries.map(([name, provider]) => (
        <div class="sub-card" key={name}>
          <div class="form-row">
            <input value={name} onBlur={(e) => rename(name, e.currentTarget.value)} />
            <select value={provider.type || "http"} onChange={(e) => update(name, { type: e.currentTarget.value })}>
              <option value="http">http</option>
              <option value="file">file</option>
              <option value="inline">inline</option>
            </select>
            <select value={provider.behavior || "domain"} onChange={(e) => update(name, { behavior: e.currentTarget.value })}>
              <option value="domain">domain</option>
              <option value="ipcidr">ipcidr</option>
              <option value="classical">classical</option>
            </select>
            <button class="danger" onClick={() => setProviders((c) => { const next = { ...c }; delete next[name]; return next; })}>删除</button>
          </div>
          <div class="form-row">
            <input placeholder="URL" class="mono" value={provider.url || ""} onInput={(e) => update(name, { url: e.currentTarget.value })} />
            <input placeholder="path" value={provider.path || ""} onInput={(e) => update(name, { path: e.currentTarget.value })} />
            <input placeholder="interval" value={provider.interval || 86400} onInput={(e) => update(name, { interval: Number(e.currentTarget.value) || 0 })} />
          </div>
        </div>
      ))}
    </Card>
  );
}

function SettingsTab({ settings, arrays, busy, onSave }) {
  const [form, setForm] = useState({ ...settings });
  useEffect(() => setForm({ ...settings }), [settings]);

  const fields = [
    ["boot_auto_start", "开机自启", "toggle"],
    ["network_mode", "网络模式", "select", ["tun", "tproxy", "ebpf", "redirect", "mixed", "enhance"]],
    ["proxy_mode", "代理模式", "select", ["core", "blacklist", "whitelist"]],
    ["ipv6", "IPv6", "toggle"],
    ["performance_mode", "性能模式", "toggle"],
    ["quic", "QUIC", "select", ["enable", "disable"]],
    ["proxy_tcp", "代理 TCP", "toggle"],
    ["proxy_udp", "代理 UDP", "toggle"],
    ["dns_hijack_mode", "DNS 劫持", "select", ["disable", "tproxy", "redirect"]],
    ["bypass_cn_ip", "绕过大陆 IP", "toggle"],
    ["cnip_mode", "CNIP 匹配", "select", ["ipset", "ebpf"]],
    ["update_subscription", "自动更新订阅", "toggle"],
    ["update_geo", "自动更新 Geo", "toggle"],
    ["use_ghproxy", "GitHub 镜像", "toggle"],
    ["url_ghproxy", "镜像地址", "text"],
    ["ruleset_prefix", "规则集前缀", "text"],
    ["cgroup_memcg", "内存限制", "toggle"],
    ["memcg_limit", "内存上限", "text"],
    ["taskset_cpu", "CPU 绑定", "toggle"],
    ["allow_cpu", "允许核心", "text"],
    ["mac_filter", "热点 MAC 过滤", "toggle"],
    ["mac_mode", "MAC 模式", "select", ["blacklist", "whitelist"]],
  ];

  const valueOf = (key) => form[key] ?? "";
  const set = (key, value) => setForm((c) => ({ ...c, [key]: value }));

  return (
    <Card title="设置" actions={<button disabled={busy} onClick={() => onSave(form)}>保存</button>}>
      <div class="settings-grid">
        {fields.map(([key, label, type, options]) => (
          <Field label={label} key={key}>
            {type === "toggle" ? (
              <input
                type="checkbox"
                checked={String(valueOf(key)) === "true"}
                onChange={(e) => set(key, e.currentTarget.checked ? "true" : "false")}
              />
            ) : type === "select" ? (
              <select value={valueOf(key)} onChange={(e) => set(key, e.currentTarget.value)}>
                {options.map((option) => <option value={option} key={option}>{option}</option>)}
              </select>
            ) : (
              <input value={valueOf(key)} onInput={(e) => set(key, e.currentTarget.value)} />
            )}
          </Field>
        ))}
      </div>
      <p class="muted">数组项（CIDR、Wi-Fi 名单）请使用命令行维护，页面暂不提供。</p>
      <pre class="mono small">{JSON.stringify(arrays, null, 2)}</pre>
    </Card>
  );
}

function LogsTab({ lines, busy, onRefresh }) {
  return (
    <Card title="日志" actions={<button disabled={busy} onClick={onRefresh}>刷新</button>}>
      <pre class="log">{lines.length ? lines.join("\n") : "暂无日志"}</pre>
    </Card>
  );
}

function DangerTab({ busy, onRun }) {
  const [confirming, setConfirming] = useState("");
  const actions = [
    ["delete-config", "删除当前配置", "会把 config.yaml 移走，核心将无法启动"],
    ["clear-logs", "清空日志", "会清空 runs.log、tool.log 和核心日志"],
    ["reset-subs", "重置订阅", "会清空订阅清单和所有 provider 文件"],
    ["upkernel-mihomo", "更新 mihomo 核心", "从模块内置 GitHub 源下载并替换核心"],
    ["upkernel-sing-box", "更新 sing-box 核心", "从模块内置 GitHub 源下载并替换核心"],
  ];

  return (
    <Card title="危险操作">
      <p class="warn">以下操作不可撤销，且不会自动备份。</p>
      {actions.map(([id, label, hint]) => (
        <div class="danger-row" key={id}>
          <div><strong>{label}</strong><p class="muted">{hint}</p></div>
          {confirming === id ? (
            <div class="form-row">
              <button class="danger" disabled={busy} onClick={async () => { await onRun(id); setConfirming(""); }}>确认执行</button>
              <button onClick={() => setConfirming("")}>取消</button>
            </div>
          ) : (
            <button class="danger" onClick={() => setConfirming(id)}>执行</button>
          )}
        </div>
      ))}
    </Card>
  );
}

export function App() {
  const [tab, setTab] = useState("status");
  const [status, setStatus] = useState(null);
  const [subs, setSubs] = useState([]);
  const [config, setConfig] = useState({ "proxy-groups": [], "rule-providers": {}, rules: [] });
  const [settings, setSettings] = useState({});
  const [arrays, setArrays] = useState({});
  const [logs, setLogs] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const wrap = async (fn) => {
    setBusy(true);
    setError("");
    try {
      await fn();
    } catch (caught) {
      setError(caught?.message || String(caught));
    } finally {
      setBusy(false);
    }
  };

  const loadStatus = () => wrap(async () => setStatus(await runJson(`${TOOL} status --json`)));
  const loadSubs = () => wrap(async () => setSubs((await runJson(`${TOOL} sub list --json`)).items || []));
  const loadConfig = () => wrap(async () => setConfig(await runJson(`${TOOL} config get --json`)));
  const loadSettings = () => wrap(async () => {
    const data = await runJson(`${TOOL} settings get --json`);
    setSettings(data.settings || {});
    setArrays(data.arrays || {});
  });
  const loadLogs = () => wrap(async () => setLogs((await runJson(`${TOOL} logs 200 --json`)).lines || []));

  useEffect(() => {
    void loadStatus();
    void loadSubs();
    void loadConfig();
    void loadSettings();
    void loadLogs();
    const timer = setInterval(() => { void loadStatus(); }, 10000);
    return () => clearInterval(timer);
  }, []);

  const serviceAction = (action) => wrap(async () => {
    await run(`${TOOL.split("/scripts/")[0]}/scripts/box.service ${action}`);
    await loadStatus();
  });

  const saveConfig = (next) => wrap(async () => {
    await run(`${TOOL} config apply ${await writeTemp(next)}`);
    setConfig(next);
  });

  const saveSettings = (form) => wrap(async () => {
    await run(`${TOOL} settings apply ${await writeTemp(form)}`);
    await loadSettings();
  });

  const runDanger = (id) => wrap(async () => {
    if (id === "delete-config") await run(`${TOOL} config delete`);
    if (id === "clear-logs") await run(`${TOOL} logs clear --json`);
    if (id === "reset-subs") await run(`${TOOL} reset subscriptions`);
    if (id === "upkernel-mihomo") await run(`${TOOL} upkernel mihomo`);
    if (id === "upkernel-sing-box") await run(`${TOOL} upkernel sing-box`);
    await loadStatus();
  });

  const tabs = [
    ["status", "概览"],
    ["subs", "订阅"],
    ["groups", "策略"],
    ["rules", "规则"],
    ["providers", "规则集"],
    ["settings", "设置"],
    ["logs", "日志"],
    ["danger", "危险"],
  ];

  return (
    <div class="app">
      <header class="top">
        <h1>Box 管理</h1>
        <button onClick={() => { void loadStatus(); void loadSubs(); void loadConfig(); void loadSettings(); void loadLogs(); }}>刷新</button>
      </header>
      <nav class="tabs">
        {tabs.map(([id, label]) => (
          <button class={tab === id ? "active" : ""} onClick={() => setTab(id)} key={id}>{label}</button>
        ))}
      </nav>
      {error ? <div class="error">{error}</div> : null}
      <main>
        {tab === "status" && <StatusTab status={status} busy={busy} onAction={serviceAction} />}
        {tab === "subs" && (
          <SubsTab
            items={subs}
            busy={busy}
            onRefresh={loadSubs}
            onAdd={(name, url) => wrap(async () => { await run(`${TOOL} sub add ${shellQuote(url)} ${shellQuote(name)}`); await loadSubs(); })}
            onRemove={(name) => wrap(async () => { await run(`${TOOL} sub rm ${shellQuote(name)}`); await loadSubs(); })}
            onUpdate={() => wrap(async () => { await run(`${TOOL} sub update`); await loadSubs(); await loadStatus(); })}
          />
        )}
        {tab === "groups" && <GroupsTab config={config} busy={busy} onSave={saveConfig} />}
        {tab === "rules" && <RulesTab config={config} busy={busy} onSave={saveConfig} />}
        {tab === "providers" && <ProvidersTab config={config} busy={busy} onSave={saveConfig} />}
        {tab === "settings" && <SettingsTab settings={settings} arrays={arrays} busy={busy} onSave={saveSettings} />}
        {tab === "logs" && <LogsTab lines={logs} busy={busy} onRefresh={loadLogs} />}
        {tab === "danger" && <DangerTab busy={busy} onRun={runDanger} />}
      </main>
    </div>
  );
}
