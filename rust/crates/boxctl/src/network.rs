use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::network_rules::{Action, Document as NetworkRuleDocument, Family, Mode, Scope};
use crate::paths::Paths;

const SYSTEM_PATH: &[&str] = &["/system/bin", "/system/xbin", "/sbin"];

#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub iptables: bool,
    pub ip6tables: bool,
    pub ip6tables_nat: bool,
    pub ip6tables_mangle: bool,
    pub ipset: bool,
    pub nft: bool,
    pub tun: bool,
    pub api_level: u32,
}

impl Capabilities {
    pub fn json(self) -> Value {
        json!({
            "iptables": self.iptables,
            "ip6tables": self.ip6tables,
            "ip6tables_nat": self.ip6tables_nat,
            "ip6tables_mangle": self.ip6tables_mangle,
            "ipset": self.ipset,
            "nft": self.nft,
            "tun": self.tun,
            "api_level": self.api_level
        })
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub family: &'static str,
    pub table: &'static str,
    pub chain: &'static str,
    pub args: Vec<String>,
}

impl Rule {
    pub fn json(&self) -> Value {
        json!({
            "command": self.family,
            "table": self.table,
            "chain": self.chain,
            "args": self.args
        })
    }
}

impl Rule {
    fn command_args(&self) -> Vec<String> {
        if self.family == "nft" || self.table.is_empty() {
            return self.args.clone();
        }
        let mut args = Vec::with_capacity(self.args.len() + 3);
        if matches!(self.family, "iptables" | "ip6tables") {
            args.push("-w".to_string());
            args.push("10".to_string());
        }
        args.push("-t".to_string());
        args.push(self.table.to_string());
        args.extend(self.args.iter().cloned());
        args
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub requested_mode: String,
    pub effective_mode: String,
    pub degraded: bool,
    pub rules: Vec<Rule>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn json(&self) -> Value {
        json!({
            "requested_mode": self.requested_mode,
            "effective_mode": self.effective_mode,
            "degraded": self.degraded,
            "rules": self.rules.iter().map(Rule::json).collect::<Vec<_>>(),
            "warnings": self.warnings
        })
    }

    fn validate(&self) -> Result<()> {
        use std::collections::{BTreeMap, BTreeSet};
        let builtins: BTreeSet<&'static str> =
            ["PREROUTING", "INPUT", "FORWARD", "OUTPUT", "POSTROUTING"]
                .into_iter()
                .collect();
        let mut chains: BTreeMap<(&'static str, &'static str, &'static str), usize> =
            BTreeMap::new();
        for rule in &self.rules {
            if rule.family == "ip" || rule.family == "nft" || rule.table.is_empty() {
                continue;
            }
            if rule.args.first().is_some_and(|value| value == "-N") {
                *chains
                    .entry((rule.family, rule.table, rule.chain))
                    .or_default() += 1;
            }
        }
        for ((family, table, chain), count) in &chains {
            if *count > 1 {
                return Err(BoxError::internal(format!(
                    "network plan defines chain {family}/{table}/{chain} more than once"
                )));
            }
        }
        for rule in &self.rules {
            if rule.family == "ip" || rule.family == "nft" || rule.table.is_empty() {
                continue;
            }
            let action = rule.args.first().map(String::as_str).unwrap_or_default();
            if action == "-A" || action == "-I" {
                let parent = rule.args.get(1).map(String::as_str).unwrap_or_default();
                if !builtins.contains(parent)
                    && !chains.contains_key(&(rule.family, rule.table, rule.chain))
                {
                    return Err(BoxError::internal(format!(
                        "network plan appends to undefined chain {}",
                        rule.chain
                    )));
                }
            }
            if let Some(index) = rule.args.iter().position(|value| value == "-j") {
                if let Some(target) = rule.args.get(index + 1) {
                    if target.starts_with("BOX_")
                        && !chains.contains_key(&(rule.family, rule.table, target.as_str()))
                    {
                        return Err(BoxError::internal(format!(
                            "network plan jumps to undefined chain {}",
                            target
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
pub trait Runner {
    fn run(&mut self, command: &str, args: &[String]) -> Result<()>;
}

#[allow(dead_code)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, command: &str, args: &[String]) -> Result<()> {
        let output = std::process::Command::new(command)
            .args(args)
            .output()
            .map_err(|error| BoxError::from(error).with_detail(format!("cannot run {command}")))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(
            BoxError::new(ErrorCode::Command, "network rule command failed").with_detail(format!(
                "{command} {}: {}",
                args.join(" "),
                stderr.trim()
            )),
        )
    }
}

pub fn apply(plan: &Plan, runner: &mut dyn Runner) -> Result<()> {
    plan.validate()?;
    let mut applied = Vec::new();
    for rule in &plan.rules {
        let args = rule.command_args();
        if let Err(error) = runner.run(rule.family, &args) {
            let original = format!(
                "{} {}: {}",
                rule.family,
                args.join(" "),
                error.detail.as_deref().unwrap_or(&error.message)
            );
            let rollback = cleanup_rules(applied.iter().rev(), runner);
            return if let Err(rollback_error) = rollback {
                Err(error.with_detail(format!(
                    "{original}; rollback failed: {}",
                    rollback_error
                        .detail
                        .as_deref()
                        .unwrap_or(&rollback_error.message)
                )))
            } else {
                Err(error.with_detail(original))
            };
        }
        applied.push(rule.clone());
    }
    Ok(())
}

pub fn cleanup(plan: &Plan, runner: &mut dyn Runner) -> Result<()> {
    let mut last = None;
    for _ in 0..3 {
        match cleanup_rules(plan.rules.iter().rev(), runner) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    Err(last
        .unwrap_or_else(|| BoxError::new(ErrorCode::Command, "network rule cleanup did not run")))
}

pub fn cleanup_all(
    settings: &Value,
    caps: Capabilities,
    dns_port: Option<u16>,
    runner: &mut dyn Runner,
) -> Result<bool> {
    let mut rules = Vec::new();
    for mode in ["tun", "tproxy", "redirect", "mixed", "enhance"] {
        let mut values = settings.clone();
        values["network_mode"] = Value::String(mode.to_string());
        if let Ok(plan) = plan_with_dns(&values, caps, dns_port) {
            rules.extend(plan.rules);
        }
    }
    if rules.is_empty() {
        return Ok(false);
    }
    cleanup_rules(rules.iter().rev(), runner)?;
    Ok(true)
}

pub fn cleanup_stale(runner: &mut dyn Runner) -> bool {
    let mut touched = false;
    let chains = [
        "BOX_TPROXY",
        "BOX_TPROXY_LOCAL",
        "BOX_LOCAL_TPROXY",
        "BOX_TUN_BYPASS_PRE",
        "BOX_TUN_BYPASS_OUT",
        "BOX_REDIRECT",
        "BOX_REDIRECT_LOCAL",
        "BOX_LOCAL_REDIRECT",
        "BOX_DNS",
        "BOX_DNS_PRE",
        "BOX_FILTER_LOCAL",
        "BOX_FILTER_PRE",
        "BOX_LOCAL_UDP",
        "BOX_QUIC_BLOCK",
    ];
    let mut run = |command: &str, args: Vec<String>| {
        if runner.run(command, &args).is_ok() {
            touched = true;
        }
    };
    for family in ["iptables", "ip6tables"] {
        for (table, parents) in [
            ("mangle", ["PREROUTING", "OUTPUT"]),
            ("nat", ["PREROUTING", "OUTPUT"]),
            ("filter", ["PREROUTING", "OUTPUT"]),
        ] {
            for chain in chains {
                for parent in parents {
                    run(
                        family,
                        vec![
                            "-w".to_string(),
                            "10".to_string(),
                            "-t".to_string(),
                            table.to_string(),
                            "-D".to_string(),
                            parent.to_string(),
                            "-j".to_string(),
                            chain.to_string(),
                        ],
                    );
                }
            }
            for chain in chains {
                run(
                    family,
                    vec![
                        "-w".to_string(),
                        "10".to_string(),
                        "-t".to_string(),
                        table.to_string(),
                        "-F".to_string(),
                        chain.to_string(),
                    ],
                );
                run(
                    family,
                    vec![
                        "-w".to_string(),
                        "10".to_string(),
                        "-t".to_string(),
                        table.to_string(),
                        "-X".to_string(),
                        chain.to_string(),
                    ],
                );
            }
        }
    }
    for family in ["iptables", "ip6tables"] {
        for chain in chains {
            run(
                family,
                vec![
                    "-w".to_string(),
                    "10".to_string(),
                    "-t".to_string(),
                    "filter".to_string(),
                    "-D".to_string(),
                    "FORWARD".to_string(),
                    "-j".to_string(),
                    chain.to_string(),
                ],
            );
        }
    }
    for ip in ["ip", "ip6"] {
        for mark in [
            "0x01000000/0x01000000",
            "0x2000000/0x2000000",
            "0x3000000/0x3000000",
        ] {
            run(
                "ip",
                vec![
                    if ip == "ip6" { "-6" } else { "-4" }.to_string(),
                    "rule".to_string(),
                    "del".to_string(),
                    "fwmark".to_string(),
                    mark.to_string(),
                ],
            );
        }
        for table in ["100", "2025"] {
            run(
                "ip",
                vec![
                    if ip == "ip6" { "-6" } else { "-4" }.to_string(),
                    "route".to_string(),
                    "flush".to_string(),
                    "table".to_string(),
                    table.to_string(),
                ],
            );
        }
    }
    touched
}

fn cleanup_rules<'a>(rules: impl Iterator<Item = &'a Rule>, runner: &mut dyn Runner) -> Result<()> {
    let mut failed = false;
    let mut first_detail = None;
    for rollback in rollback_rules(rules) {
        if let Err(error) = runner.run(rollback.family, &rollback.command_args()) {
            if !already_absent(&error) {
                failed = true;
                first_detail.get_or_insert_with(|| {
                    format!(
                        "{} {}: {}",
                        rollback.family,
                        rollback.command_args().join(" "),
                        error.detail.as_deref().unwrap_or(&error.message)
                    )
                });
            }
        }
    }
    if failed {
        return Err(
            BoxError::new(ErrorCode::Command, "network rule cleanup failed")
                .with_detail(first_detail.unwrap_or_default()),
        );
    }
    Ok(())
}

fn already_absent(error: &BoxError) -> bool {
    let text = format!(
        "{} {}",
        error.message.to_lowercase(),
        error.detail.as_deref().unwrap_or_default().to_lowercase()
    );
    [
        "no chain/target/match",
        "couldn't find target",
        "could not find target",
        "bad rule",
        "no such file or directory",
        "cannot find device",
        "no such process",
        "invalid argument",
        "does not exist",
        "not found",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

#[allow(dead_code)]
fn rollback_rules<'a>(rules: impl Iterator<Item = &'a Rule>) -> Vec<Rule> {
    let mut result = Vec::new();
    for rule in rules {
        let Some(action) = rule.args.first().map(String::as_str) else {
            continue;
        };
        match action {
            "-A" | "-I" => {
                let mut args = rule.args.clone();
                args[0] = "-D".to_string();
                result.push(Rule {
                    family: rule.family,
                    table: rule.table,
                    chain: rule.chain,
                    args,
                });
            }
            "-N" => {
                result.push(Rule {
                    family: rule.family,
                    table: rule.table,
                    chain: rule.chain,
                    args: vec!["-F".to_string(), rule.chain.to_string()],
                });
                result.push(Rule {
                    family: rule.family,
                    table: rule.table,
                    chain: rule.chain,
                    args: vec!["-X".to_string(), rule.chain.to_string()],
                });
            }
            "rule" | "route" if rule.args.get(1).is_some_and(|value| value == "add") => {
                let mut args = rule.args.clone();
                args[1] = "del".to_string();
                result.push(Rule {
                    family: rule.family,
                    table: rule.table,
                    chain: rule.chain,
                    args,
                });
            }
            "-4" | "-6"
                if rule
                    .args
                    .get(1)
                    .is_some_and(|value| value == "rule" || value == "route")
                    && rule.args.get(2).is_some_and(|value| value == "add") =>
            {
                let mut args = rule.args.clone();
                args[2] = "del".to_string();
                result.push(Rule {
                    family: rule.family,
                    table: rule.table,
                    chain: rule.chain,
                    args,
                });
            }
            "add"
                if rule.family == "nft"
                    && rule.args.get(1).is_some_and(|value| value == "table") =>
            {
                let mut args = rule.args.clone();
                args[0] = "delete".to_string();
                result.push(Rule {
                    family: rule.family,
                    table: rule.table,
                    chain: rule.chain,
                    args,
                });
            }
            _ => {}
        }
    }
    result
}

static CAPABILITY_CACHE: OnceLock<Mutex<HashMap<PathBuf, Capabilities>>> = OnceLock::new();

pub fn system_protection_summary() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "id": "system-core-bypass",
            "name": "核心进程旁路",
            "description": "核心和 root 流量访问本机代理端口时直连，避免回环",
            "scopes": ["local"],
            "readOnly": true
        }),
        serde_json::json!({
            "id": "system-local-destination",
            "name": "本机与局域网目标",
            "description": "由默认 CIDR 规则和 ADB 端口规则覆盖；内核无 addrtype 时由用户可见规则表达",
            "scopes": ["local", "forward"],
            "readOnly": true
        }),
        serde_json::json!({
            "id": "system-dns-chains",
            "name": "DNS 链结构",
            "description": "BOX_DNS 处理本机 DNS，BOX_DNS_PRE 处理转发 DNS",
            "scopes": ["dns"],
            "readOnly": true
        }),
        serde_json::json!({
            "id": "system-ip-rules",
            "name": "策略路由",
            "description": "tproxy/TUN 的 mark、ip rule 和本地路由由模块固定管理",
            "scopes": ["forward"],
            "readOnly": true
        }),
    ]
}

pub fn warm_capabilities(paths: &Paths) -> Capabilities {
    let capabilities = detect(paths);
    let cache = CAPABILITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(paths.home.clone(), capabilities);
    }
    capabilities
}

pub fn cached_capabilities(paths: &Paths) -> Option<Capabilities> {
    CAPABILITY_CACHE
        .get()
        .and_then(|cache| cache.lock().ok())
        .and_then(|guard| guard.get(&paths.home).copied())
}

pub fn detect(_paths: &Paths) -> Capabilities {
    Capabilities {
        iptables: has_command("iptables"),
        ip6tables: has_command("ip6tables"),
        ip6tables_nat: has_table("ip6tables", "nat"),
        ip6tables_mangle: has_table("ip6tables", "mangle"),
        ipset: has_command("ipset"),
        nft: has_command("nft"),
        tun: Path::new("/dev/net/tun").exists(),
        api_level: api_level(),
    }
}

pub fn plan(settings: &Value, caps: Capabilities) -> Result<Plan> {
    plan_with_dns(settings, caps, None)
}

pub fn plan_with_dns(settings: &Value, caps: Capabilities, dns_port: Option<u16>) -> Result<Plan> {
    plan_with_dns_and_rules(settings, caps, dns_port, None)
}

pub fn plan_with_dns_and_rules(
    settings: &Value,
    caps: Capabilities,
    dns_port: Option<u16>,
    rules: Option<&NetworkRuleDocument>,
) -> Result<Plan> {
    let mode = settings
        .get("network_mode")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !matches!(mode, "tun" | "tproxy" | "redirect" | "mixed" | "enhance") {
        return Err(BoxError::validation(format!(
            "unknown network mode: {mode}"
        )));
    }
    let tproxy_port = port(settings, "tproxy_port")?;
    let redir_port = port(settings, "redir_port")?;
    let ipv6 = settings
        .get("ipv6")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    let proxy_tcp = enabled(settings, "proxy_tcp");
    let proxy_udp = enabled(settings, "proxy_udp");
    let force_v4 = string_list(settings, "force_proxy_cidrs");
    let force_v6 = string_list(settings, "force_proxy_cidrs6");
    let bypass_cn = enabled(settings, "bypass_cn_ip");
    let cnip_ipset = bypass_cn && caps.ipset;
    let force_uids = if cnip_ipset {
        cnip_force_proxy_uids(settings)
    } else {
        Vec::new()
    };
    let mut plan = Plan {
        requested_mode: mode.to_string(),
        effective_mode: mode.to_string(),
        degraded: false,
        rules: Vec::new(),
        warnings: Vec::new(),
    };
    if bypass_cn && !caps.ipset {
        plan.warnings
            .push("ipset is unavailable; CNIP bypass is degraded".to_string());
        plan.degraded = true;
    }
    if ipv6 && !caps.ip6tables {
        plan.warnings
            .push("ip6tables is unavailable; IPv6 rules are skipped".to_string());
        plan.degraded = true;
    }
    if mode == "tun" {
        if !caps.tun {
            plan.warnings
                .push("TUN device is unavailable; cannot apply TUN mode".to_string());
            plan.degraded = true;
        }
        if caps.iptables {
            if let Some(port) = dns_port {
                let hijack_udp = enabled(settings, "dns_hijack_udp");
                let hijack_tcp = enabled(settings, "dns_hijack_tcp");
                if hijack_udp || hijack_tcp {
                    plan.rules.extend(dns_rules(
                        "iptables", port, hijack_udp, hijack_tcp, rules, mode,
                    ));
                }
            }
            plan.rules.extend(tun_bypass_rules(
                "iptables",
                settings,
                cnip_ipset.then_some("cnip"),
                &force_v4,
                rules,
                mode,
            ));
            plan.rules.extend(quic_block_rules(settings));
            plan.rules
                .extend(user_filter_rules(rules, mode, "iptables"));
            if ipv6 && caps.ip6tables {
                plan.rules
                    .extend(user_filter_rules(rules, mode, "ip6tables"));
            }
        } else if caps.nft {
            if let Some(port) = dns_port {
                let hijack_udp = enabled(settings, "dns_hijack_udp");
                let hijack_tcp = enabled(settings, "dns_hijack_tcp");
                if hijack_udp || hijack_tcp {
                    plan.rules
                        .extend(nft_dns_rules(port, hijack_udp, hijack_tcp));
                }
            }
        }
        return Ok(plan);
    }
    if !caps.iptables && caps.nft {
        return nft_plan(&mut plan, settings, caps, dns_port);
    }
    if !caps.iptables {
        plan.warnings
            .push("iptables is unavailable; no rules can be applied".to_string());
        plan.degraded = true;
        return Ok(plan);
    }
    let effective = plan.effective_mode.as_str();
    if cnip_ipset && caps.iptables {
        plan.rules.extend(ipset_create_rules(ipv6));
    }
    if matches!(effective, "tproxy" | "enhance") {
        plan.rules.extend(tproxy_rules(
            "iptables",
            tproxy_port,
            proxy_tcp,
            proxy_udp,
            cnip_ipset.then_some("cnip"),
            &force_v4,
            rules,
            mode,
        ));
        if ipv6 && caps.ip6tables && caps.ip6tables_mangle {
            plan.rules.extend(tproxy_rules(
                "ip6tables",
                tproxy_port,
                proxy_tcp,
                proxy_udp,
                cnip_ipset.then_some("cnip6"),
                &force_v6,
                rules,
                mode,
            ));
        } else if ipv6 && caps.ip6tables && !caps.ip6tables_mangle {
            plan.warnings.push(
                "ip6tables mangle table is unavailable; IPv6 tproxy rules are skipped".to_string(),
            );
            plan.degraded = true;
        }
    }
    if matches!(effective, "redirect" | "mixed" | "enhance") {
        plan.rules.extend(redirect_rules(
            "iptables",
            redir_port,
            proxy_tcp,
            cnip_ipset.then_some("cnip"),
            &force_v4,
            rules,
            mode,
        ));
        if ipv6 && caps.ip6tables && caps.ip6tables_nat {
            plan.rules.extend(redirect_rules(
                "ip6tables",
                redir_port,
                proxy_tcp,
                cnip_ipset.then_some("cnip6"),
                &force_v6,
                rules,
                mode,
            ));
        } else if ipv6 && caps.ip6tables && !caps.ip6tables_nat {
            plan.warnings.push(
                "ip6tables nat table is unavailable; IPv6 redirect rules are skipped".to_string(),
            );
            plan.degraded = true;
        }
    }
    let proxy_mode = settings
        .get("proxy_mode")
        .and_then(Value::as_str)
        .unwrap_or("core");
    let proxy_uids = settings
        .get("__proxy_uids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u32::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let proxy_gids = settings
        .get("__proxy_gids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u32::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let local_force_active = !force_uids.is_empty() && cnip_ipset;
    if !local_force_active && matches!(effective, "tproxy" | "redirect" | "mixed" | "enhance") {
        plan.rules.extend(local_proxy_rules(
            "iptables",
            "nat",
            "BOX_LOCAL_REDIRECT",
            "redirect",
            redir_port,
            proxy_tcp,
            false,
            proxy_mode,
            &proxy_uids,
            &proxy_gids,
            rules,
            mode,
        ));
    }
    if matches!(effective, "tproxy" | "enhance") && proxy_udp {
        plan.rules.extend(local_udp_tproxy_rules(
            "iptables",
            tproxy_port,
            proxy_mode,
            &proxy_uids,
            &proxy_gids,
            rules,
            mode,
        ));
    } else if effective == "redirect" && proxy_udp {
        plan.warnings.push(
            "local UDP transparent proxy is unavailable in redirect; local UDP remains direct"
                .to_string(),
        );
        plan.degraded = true;
    }
    plan.rules.extend(quic_block_rules(settings));
    if effective == "mixed" {
        plan.rules.extend(tun_bypass_rules(
            "iptables",
            settings,
            cnip_ipset.then_some("cnip"),
            &force_v4,
            rules,
            mode,
        ));
    }
    if let Some(port) = dns_port {
        let hijack_udp = enabled(settings, "dns_hijack_udp");
        let hijack_tcp = enabled(settings, "dns_hijack_tcp");
        if hijack_udp || hijack_tcp {
            plan.rules.extend(dns_rules(
                "iptables", port, hijack_udp, hijack_tcp, rules, mode,
            ));
        }
    }
    if !force_uids.is_empty() && cnip_ipset {
        if matches!(effective, "tproxy" | "enhance") {
            plan.rules.extend(tproxy_local_force_rules(
                "iptables",
                "cnip",
                &force_uids,
                tproxy_port,
                proxy_tcp,
                proxy_udp,
            ));
            if ipv6 && caps.ip6tables && caps.ip6tables_mangle {
                plan.rules.extend(tproxy_local_force_rules(
                    "ip6tables",
                    "cnip6",
                    &force_uids,
                    tproxy_port,
                    proxy_tcp,
                    proxy_udp,
                ));
            }
        }
        if matches!(effective, "redirect" | "mixed" | "enhance") {
            plan.rules.extend(redirect_local_force_rules(
                "iptables",
                "cnip",
                &force_uids,
                redir_port,
                proxy_tcp,
            ));
            if ipv6 && caps.ip6tables && caps.ip6tables_nat {
                plan.rules.extend(redirect_local_force_rules(
                    "ip6tables",
                    "cnip6",
                    &force_uids,
                    redir_port,
                    proxy_tcp,
                ));
            }
        }
    }
    plan.rules
        .extend(user_filter_rules(rules, mode, "iptables"));
    if ipv6 && caps.ip6tables {
        plan.rules
            .extend(user_filter_rules(rules, mode, "ip6tables"));
    }
    Ok(plan)
}

fn nft_plan(
    plan: &mut Plan,
    settings: &Value,
    caps: Capabilities,
    dns_port: Option<u16>,
) -> Result<Plan> {
    let tproxy_port = port(settings, "tproxy_port")?;
    let redir_port = port(settings, "redir_port")?;
    let proxy_tcp = enabled(settings, "proxy_tcp");
    let proxy_udp = enabled(settings, "proxy_udp");
    let ipv6 = settings
        .get("ipv6")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    let mut rules = vec![Rule {
        family: "nft",
        table: "",
        chain: "",
        args: vec![
            "add".to_string(),
            "table".to_string(),
            "inet".to_string(),
            "box".to_string(),
        ],
    }];
    let tproxy = matches!(plan.effective_mode.as_str(), "tproxy" | "mixed" | "enhance");
    let redirect = matches!(
        plan.effective_mode.as_str(),
        "redirect" | "mixed" | "enhance"
    );
    if tproxy {
        rules.push(nft_chain(
            "box_tproxy",
            "filter hook prerouting priority mangle",
        ));
        rules.push(nft_rule("box_tproxy", "ip daddr 0.0.0.0/8 return"));
        rules.push(nft_rule("box_tproxy", "ip daddr 10.0.0.0/8 return"));
        rules.push(nft_rule("box_tproxy", "ip daddr 127.0.0.0/8 return"));
        rules.push(nft_rule("box_tproxy", "ip daddr 169.254.0.0/16 return"));
        rules.push(nft_rule("box_tproxy", "ip daddr 172.16.0.0/12 return"));
        rules.push(nft_rule("box_tproxy", "ip daddr 192.168.0.0/16 return"));
        rules.push(nft_rule("box_tproxy", "ip daddr 224.0.0.0/4 return"));
        rules.push(nft_rule(
            "box_tproxy",
            "meta mark 0x01000000/0x01000000 return",
        ));
        if proxy_tcp {
            rules.push(nft_rule(
                "box_tproxy",
                &format!("meta nfproto ipv4 meta l4proto tcp tproxy to :{tproxy_port}"),
            ));
        }
        if proxy_udp {
            rules.push(nft_rule(
                "box_tproxy",
                &format!("meta nfproto ipv4 meta l4proto udp tproxy to :{tproxy_port}"),
            ));
        }
        rules.push(Rule {
            family: "ip",
            table: "",
            chain: "",
            args: vec![
                "-4".to_string(),
                "rule".to_string(),
                "add".to_string(),
                "fwmark".to_string(),
                "0x01000000/0x01000000".to_string(),
                "table".to_string(),
                "100".to_string(),
            ],
        });
        rules.push(Rule {
            family: "ip",
            table: "",
            chain: "",
            args: vec![
                "-4".to_string(),
                "route".to_string(),
                "add".to_string(),
                "local".to_string(),
                "0.0.0.0/0".to_string(),
                "dev".to_string(),
                "lo".to_string(),
                "table".to_string(),
                "100".to_string(),
            ],
        });
        if ipv6 {
            rules.push(nft_rule("box_tproxy", "ip6 daddr ::1/128 return"));
            rules.push(nft_rule("box_tproxy", "ip6 daddr fc00::/7 return"));
            rules.push(nft_rule("box_tproxy", "ip6 daddr fe80::/10 return"));
            rules.push(Rule {
                family: "ip",
                table: "",
                chain: "",
                args: vec![
                    "-6".to_string(),
                    "rule".to_string(),
                    "add".to_string(),
                    "fwmark".to_string(),
                    "0x01000000/0x01000000".to_string(),
                    "table".to_string(),
                    "100".to_string(),
                ],
            });
            rules.push(Rule {
                family: "ip",
                table: "",
                chain: "",
                args: vec![
                    "-6".to_string(),
                    "route".to_string(),
                    "add".to_string(),
                    "local".to_string(),
                    "::/0".to_string(),
                    "dev".to_string(),
                    "lo".to_string(),
                    "table".to_string(),
                    "100".to_string(),
                ],
            });
            if proxy_tcp {
                rules.push(nft_rule(
                    "box_tproxy",
                    &format!("meta nfproto ipv6 meta l4proto tcp tproxy to [::]:{tproxy_port}"),
                ));
            }
            if proxy_udp {
                rules.push(nft_rule(
                    "box_tproxy",
                    &format!("meta nfproto ipv6 meta l4proto udp tproxy to [::]:{tproxy_port}"),
                ));
            }
        }
    }
    if redirect {
        rules.push(nft_chain(
            "box_redirect",
            "nat hook prerouting priority dstnat",
        ));
        rules.push(nft_rule("box_redirect", "ip daddr 0.0.0.0/8 return"));
        rules.push(nft_rule("box_redirect", "ip daddr 10.0.0.0/8 return"));
        rules.push(nft_rule("box_redirect", "ip daddr 127.0.0.0/8 return"));
        rules.push(nft_rule("box_redirect", "ip daddr 169.254.0.0/16 return"));
        rules.push(nft_rule("box_redirect", "ip daddr 172.16.0.0/12 return"));
        rules.push(nft_rule("box_redirect", "ip daddr 192.168.0.0/16 return"));
        rules.push(nft_rule("box_redirect", "ip daddr 224.0.0.0/4 return"));
        if proxy_tcp {
            rules.push(nft_rule(
                "box_redirect",
                &format!("meta nfproto ipv4 meta l4proto tcp redirect to :{redir_port}"),
            ));
        }
        if ipv6 {
            rules.push(nft_rule("box_redirect", "ip6 daddr ::1/128 return"));
            rules.push(nft_rule("box_redirect", "ip6 daddr fc00::/7 return"));
            rules.push(nft_rule("box_redirect", "ip6 daddr fe80::/10 return"));
            if proxy_tcp {
                rules.push(nft_rule(
                    "box_redirect",
                    &format!("meta nfproto ipv6 meta l4proto tcp redirect to :{redir_port}"),
                ));
            }
        }
    }
    if let Some(port) = dns_port {
        let hijack_udp = enabled(settings, "dns_hijack_udp");
        let hijack_tcp = enabled(settings, "dns_hijack_tcp");
        if hijack_udp || hijack_tcp {
            rules.extend(nft_dns_rules(port, hijack_udp, hijack_tcp));
        }
    }
    plan.rules = rules;
    if !caps.nft {
        plan.degraded = true;
        plan.warnings.push("nft is unavailable".to_string());
    }
    Ok(plan.clone())
}

fn nft_chain(name: &str, hook: &str) -> Rule {
    Rule {
        family: "nft",
        table: "",
        chain: "",
        args: vec![
            "add".to_string(),
            "chain".to_string(),
            "inet".to_string(),
            "box".to_string(),
            name.to_string(),
            format!("{{ type {hook}; policy accept; }}"),
        ],
    }
}

fn nft_rule(chain: &str, expression: &str) -> Rule {
    let mut args = vec![
        "add".to_string(),
        "rule".to_string(),
        "inet".to_string(),
        "box".to_string(),
        chain.to_string(),
    ];
    args.extend(expression.split_whitespace().map(str::to_string));
    Rule {
        family: "nft",
        table: "",
        chain: "",
        args,
    }
}

fn nft_dns_rules(port: u16, udp: bool, tcp: bool) -> Vec<Rule> {
    let mut rules = vec![nft_chain("box_dns", "nat hook output priority dstnat")];
    rules.push(nft_rule("box_dns", "meta skuid 0 return"));
    if udp {
        rules.push(nft_rule(
            "box_dns",
            &format!("udp dport 53 redirect to :{port}"),
        ));
    }
    if tcp {
        rules.push(nft_rule(
            "box_dns",
            &format!("tcp dport 53 redirect to :{port}"),
        ));
    }
    rules
}

fn dns_rules(
    family: &'static str,
    port: u16,
    udp: bool,
    tcp: bool,
    document: Option<&NetworkRuleDocument>,
    mode: &str,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    for (chain, owner) in [("BOX_DNS", true), ("BOX_DNS_PRE", false)] {
        rules.push(Rule {
            family,
            table: "nat",
            chain,
            args: vec!["-N".to_string(), chain.to_string()],
        });
        if owner {
            rules.push(Rule {
                family,
                table: "nat",
                chain,
                args: vec![
                    "-A".to_string(),
                    chain.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    "0".to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            });
        }
        rules.extend(user_rules_for(
            document,
            mode,
            family,
            Scope::Dns,
            "nat",
            chain,
        ));
        for (protocol, enabled) in [("udp", udp), ("tcp", tcp)] {
            if !enabled {
                continue;
            }
            rules.push(Rule {
                family,
                table: "nat",
                chain,
                args: vec![
                    "-A".to_string(),
                    chain.to_string(),
                    "-p".to_string(),
                    protocol.to_string(),
                    "--dport".to_string(),
                    "53".to_string(),
                    "-j".to_string(),
                    "REDIRECT".to_string(),
                    "--to-ports".to_string(),
                    port.to_string(),
                ],
            });
        }
    }
    for (parent, target) in [("OUTPUT", "BOX_DNS"), ("PREROUTING", "BOX_DNS_PRE")] {
        rules.push(Rule {
            family,
            table: "nat",
            chain: parent,
            args: vec![
                "-A".to_string(),
                parent.to_string(),
                "-j".to_string(),
                target.to_string(),
            ],
        });
    }
    rules
}

fn ipset_create_rules(ipv6: bool) -> Vec<Rule> {
    let mut rules = vec![Rule {
        family: "ipset",
        table: "",
        chain: "",
        args: vec![
            "create".to_string(),
            "cnip".to_string(),
            "hash:net".to_string(),
            "family".to_string(),
            "inet".to_string(),
            "hashsize".to_string(),
            "8192".to_string(),
            "maxelem".to_string(),
            "65536".to_string(),
            "-exist".to_string(),
        ],
    }];
    if ipv6 {
        rules.push(Rule {
            family: "ipset",
            table: "",
            chain: "",
            args: vec![
                "create".to_string(),
                "cnip6".to_string(),
                "hash:net".to_string(),
                "family".to_string(),
                "inet6".to_string(),
                "hashsize".to_string(),
                "8192".to_string(),
                "maxelem".to_string(),
                "65536".to_string(),
                "-exist".to_string(),
            ],
        });
    }
    rules
}

fn tproxy_local_force_rules(
    family: &'static str,
    set: &'static str,
    uids: &[u32],
    port: u16,
    proxy_tcp: bool,
    proxy_udp: bool,
) -> Vec<Rule> {
    let chain = "BOX_LOCAL_TPROXY";
    let mut rules = vec![
        Rule {
            family,
            table: "mangle",
            chain,
            args: vec!["-N".to_string(), chain.to_string()],
        },
        Rule {
            family,
            table: "mangle",
            chain: "OUTPUT",
            args: vec![
                "-A".to_string(),
                "OUTPUT".to_string(),
                "-j".to_string(),
                chain.to_string(),
            ],
        },
    ];
    for uid in uids {
        for (proto, enabled) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled {
                continue;
            }
            rules.push(Rule {
                family,
                table: "mangle",
                chain,
                args: vec![
                    "-A".to_string(),
                    chain.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    uid.to_string(),
                    "-m".to_string(),
                    "set".to_string(),
                    "--match-set".to_string(),
                    set.to_string(),
                    "dst".to_string(),
                    "-p".to_string(),
                    proto.to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    "0x01000000/0x01000000".to_string(),
                ],
            });
            rules.push(Rule {
                family,
                table: "mangle",
                chain,
                args: vec![
                    "-A".to_string(),
                    chain.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    uid.to_string(),
                    "-m".to_string(),
                    "set".to_string(),
                    "--match-set".to_string(),
                    set.to_string(),
                    "dst".to_string(),
                    "-p".to_string(),
                    proto.to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            });
        }
    }
    let _ = port;
    rules
}

fn redirect_local_force_rules(
    family: &'static str,
    set: &'static str,
    uids: &[u32],
    port: u16,
    proxy_tcp: bool,
) -> Vec<Rule> {
    let chain = "BOX_LOCAL_REDIRECT";
    let mut rules = vec![
        Rule {
            family,
            table: "nat",
            chain,
            args: vec!["-N".to_string(), chain.to_string()],
        },
        Rule {
            family,
            table: "nat",
            chain: "OUTPUT",
            args: vec![
                "-A".to_string(),
                "OUTPUT".to_string(),
                "-j".to_string(),
                chain.to_string(),
            ],
        },
    ];
    if proxy_tcp {
        for uid in uids {
            rules.push(Rule {
                family,
                table: "nat",
                chain,
                args: vec![
                    "-A".to_string(),
                    chain.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    uid.to_string(),
                    "-m".to_string(),
                    "set".to_string(),
                    "--match-set".to_string(),
                    set.to_string(),
                    "dst".to_string(),
                    "-p".to_string(),
                    "tcp".to_string(),
                    "-j".to_string(),
                    "REDIRECT".to_string(),
                    "--to-ports".to_string(),
                    port.to_string(),
                ],
            });
        }
    }
    rules
}

fn user_rules_for(
    document: Option<&NetworkRuleDocument>,
    mode: &str,
    family: &'static str,
    scope: Scope,
    table: &'static str,
    chain: &'static str,
) -> Vec<Rule> {
    let Some(document) = document else {
        return Vec::new();
    };
    let Some(mode) = mode_from_str(mode) else {
        return Vec::new();
    };
    let mut rules = Vec::new();
    for rule in document.rules.iter().filter(|rule| rule.enabled) {
        if !rule.modes.contains(&mode) || !rule.scopes.contains(&scope) {
            continue;
        }
        match (rule.action, table) {
            (Action::Bypass, "filter") | (Action::Reject, "mangle" | "nat") => continue,
            _ => {}
        }
        if scope == Scope::Dns && rule.action != Action::Bypass {
            continue;
        }
        if rule.family == Family::Ipv4 && family != "iptables" {
            continue;
        }
        if rule.family == Family::Ipv6 && family != "ip6tables" {
            continue;
        }
        if rule.match_fields.is_empty() {
            continue;
        }
        let mut variants: Vec<Vec<String>> = vec![Vec::new()];
        let dst_cidrs = filter_cidrs(&rule.match_fields.dst_cidrs, family);
        let src_cidrs = filter_cidrs(&rule.match_fields.src_cidrs, family);
        if !rule.match_fields.dst_cidrs.is_empty() && dst_cidrs.is_empty() {
            continue;
        }
        if !rule.match_fields.src_cidrs.is_empty() && src_cidrs.is_empty() {
            continue;
        }
        add_values(&mut variants, "-d", &dst_cidrs);
        add_values(&mut variants, "-s", &src_cidrs);
        if chain == "OUTPUT" {
            add_values(&mut variants, "-o", &rule.match_fields.dst_ifaces);
        } else {
            add_values(&mut variants, "-i", &rule.match_fields.src_ifaces);
            add_values(&mut variants, "-o", &rule.match_fields.dst_ifaces);
        }
        if !rule.match_fields.uids.is_empty() || !rule.match_fields.gids.is_empty() {
            if chain != "OUTPUT" {
                continue;
            }
            let uids = rule
                .match_fields
                .uids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>();
            let gids = rule
                .match_fields
                .gids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>();
            add_owner_values(&mut variants, "--uid-owner", &uids);
            add_owner_values(&mut variants, "--gid-owner", &gids);
        }
        let types = rule
            .match_fields
            .dst_types
            .iter()
            .map(|value| match value {
                crate::network_rules::DstType::Local => "LOCAL",
                crate::network_rules::DstType::Multicast => "MULTICAST",
                crate::network_rules::DstType::Broadcast => "BROADCAST",
            })
            .collect::<Vec<_>>();
        if !types.is_empty() {
            let mut next = Vec::new();
            for variant in &variants {
                for value in &types {
                    let mut args = variant.clone();
                    args.extend([
                        "-m".to_string(),
                        "addrtype".to_string(),
                        "--dst-type".to_string(),
                        (*value).to_string(),
                    ]);
                    next.push(args);
                }
            }
            variants = next;
        }
        let protocols = match rule.protocol {
            crate::network_rules::Protocol::Any
                if !rule.match_fields.dst_ports.is_empty()
                    || !rule.match_fields.src_ports.is_empty() =>
            {
                vec!["tcp", "udp"]
            }
            crate::network_rules::Protocol::Any => vec![],
            crate::network_rules::Protocol::Tcp => vec!["tcp"],
            crate::network_rules::Protocol::Udp => vec!["udp"],
        };
        let needs_protocol = !protocols.is_empty();
        if needs_protocol {
            let mut next = Vec::new();
            for variant in &variants {
                for protocol in &protocols {
                    let mut args = variant.clone();
                    args.extend(["-p".to_string(), (*protocol).to_string()]);
                    next.push(args);
                }
            }
            variants = next;
        }
        add_values(&mut variants, "--dport", &rule.match_fields.dst_ports);
        add_values(&mut variants, "--sport", &rule.match_fields.src_ports);
        for args in variants {
            let mut args = args;
            args.extend([
                "-j".to_string(),
                match rule.action {
                    Action::Bypass => "RETURN".to_string(),
                    Action::Reject => "REJECT".to_string(),
                },
            ]);
            let mut full = vec!["-A".to_string(), chain.to_string()];
            full.extend(args);
            rules.push(Rule {
                family,
                table,
                chain,
                args: full,
            });
        }
    }
    rules
}

fn add_values(variants: &mut Vec<Vec<String>>, flag: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let mut next = Vec::new();
    for variant in variants.iter() {
        for value in values {
            let mut args = variant.clone();
            args.extend([flag.to_string(), value.clone()]);
            next.push(args);
        }
    }
    *variants = next;
}

fn add_owner_values(variants: &mut Vec<Vec<String>>, flag: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let mut next = Vec::new();
    for variant in variants.iter() {
        for value in values {
            let mut args = variant.clone();
            args.extend([
                "-m".to_string(),
                "owner".to_string(),
                flag.to_string(),
                value.clone(),
            ]);
            next.push(args);
        }
    }
    *variants = next;
}

fn filter_cidrs(cidrs: &[String], family: &str) -> Vec<String> {
    cidrs
        .iter()
        .filter(|cidr| {
            let address = cidr.split('/').next().unwrap_or_default();
            let ipv4 = address
                .parse::<std::net::IpAddr>()
                .map(|value| value.is_ipv4())
                .unwrap_or(true);
            (family == "iptables" && ipv4) || (family == "ip6tables" && !ipv4)
        })
        .cloned()
        .collect()
}

fn mode_from_str(value: &str) -> Option<Mode> {
    Some(match value {
        "tun" => Mode::Tun,
        "tproxy" => Mode::Tproxy,
        "redirect" => Mode::Redirect,
        "mixed" => Mode::Mixed,
        "enhance" => Mode::Enhance,
        _ => return None,
    })
}

fn user_filter_rules(
    document: Option<&NetworkRuleDocument>,
    mode: &str,
    family: &'static str,
) -> Vec<Rule> {
    let Some(document) = document else {
        return Vec::new();
    };
    let local = user_rules_for(
        Some(document),
        mode,
        family,
        Scope::Local,
        "filter",
        "BOX_FILTER_LOCAL",
    );
    let forward = user_rules_for(
        Some(document),
        mode,
        family,
        Scope::Forward,
        "filter",
        "BOX_FILTER_PRE",
    );
    let filter = user_rules_for(
        Some(document),
        mode,
        family,
        Scope::Filter,
        "filter",
        "BOX_FILTER_PRE",
    );
    if local.is_empty() && forward.is_empty() && filter.is_empty() {
        return Vec::new();
    }
    let mut rules = Vec::new();
    if !local.is_empty() {
        rules.push(Rule {
            family,
            table: "filter",
            chain: "BOX_FILTER_LOCAL",
            args: vec!["-N".to_string(), "BOX_FILTER_LOCAL".to_string()],
        });
        rules.push(Rule {
            family,
            table: "filter",
            chain: "OUTPUT",
            args: vec![
                "-A".to_string(),
                "OUTPUT".to_string(),
                "-j".to_string(),
                "BOX_FILTER_LOCAL".to_string(),
            ],
        });
        rules.extend(local);
    }
    if !forward.is_empty() || !filter.is_empty() {
        rules.push(Rule {
            family,
            table: "filter",
            chain: "BOX_FILTER_PRE",
            args: vec!["-N".to_string(), "BOX_FILTER_PRE".to_string()],
        });
        rules.push(Rule {
            family,
            table: "filter",
            chain: "PREROUTING",
            args: vec![
                "-A".to_string(),
                "PREROUTING".to_string(),
                "-j".to_string(),
                "BOX_FILTER_PRE".to_string(),
            ],
        });
        rules.extend(forward);
        rules.extend(filter);
    }
    rules
}

fn local_core_bypass_rules(
    family: &'static str,
    table: &'static str,
    chain: &'static str,
) -> Vec<Rule> {
    let rules = vec![
        Rule {
            family,
            table,
            chain,
            args: vec!["-N".to_string(), chain.to_string()],
        },
        Rule {
            family,
            table,
            chain: "OUTPUT",
            args: vec![
                "-A".to_string(),
                "OUTPUT".to_string(),
                "-j".to_string(),
                chain.to_string(),
            ],
        },
        Rule {
            family,
            table,
            chain,
            args: vec![
                "-A".to_string(),
                chain.to_string(),
                "-m".to_string(),
                "owner".to_string(),
                "--uid-owner".to_string(),
                "0".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        },
    ];
    rules
}

fn tun_bypass_rules(
    family: &'static str,
    settings: &Value,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
    document: Option<&NetworkRuleDocument>,
    mode: &str,
) -> Vec<Rule> {
    let pre = "BOX_TUN_BYPASS_PRE";
    let out = "BOX_TUN_BYPASS_OUT";
    let route_mark = "0x3000000/0x3000000";
    let bypass_mark = "0x2000000/0x2000000";
    let tun_device = "meta";
    let mut rules = Vec::new();
    for chain in [pre, out] {
        rules.push(Rule {
            family,
            table: "mangle",
            chain,
            args: vec!["-N".to_string(), chain.to_string()],
        });
    }
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "PREROUTING",
        args: vec![
            "-A".to_string(),
            "PREROUTING".to_string(),
            "-j".to_string(),
            pre.to_string(),
        ],
    });
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "OUTPUT",
        args: vec![
            "-A".to_string(),
            "OUTPUT".to_string(),
            "-j".to_string(),
            out.to_string(),
        ],
    });
    let mark = |chain: &'static str, args: Vec<String>| Rule {
        family,
        table: "mangle",
        chain,
        args,
    };
    rules.push(mark(
        pre,
        vec![
            "-A".to_string(),
            pre.to_string(),
            "-i".to_string(),
            tun_device.to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    ));
    for proto in ["tcp", "udp"] {
        if (proto == "tcp" && enabled(settings, "dns_hijack_tcp"))
            || (proto == "udp" && enabled(settings, "dns_hijack_udp"))
        {
            rules.push(mark(
                pre,
                vec![
                    "-A".to_string(),
                    pre.to_string(),
                    "-p".to_string(),
                    proto.to_string(),
                    "--dport".to_string(),
                    "53".to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    route_mark.to_string(),
                ],
            ));
            rules.push(mark(
                pre,
                vec![
                    "-A".to_string(),
                    pre.to_string(),
                    "-p".to_string(),
                    proto.to_string(),
                    "--dport".to_string(),
                    "53".to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            ));
        }
    }
    for cidr in force_cidrs {
        for proto in ["tcp", "udp"] {
            rules.push(mark(
                pre,
                vec![
                    "-A".to_string(),
                    pre.to_string(),
                    "-d".to_string(),
                    cidr.clone(),
                    "-p".to_string(),
                    proto.to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    route_mark.to_string(),
                ],
            ));
            rules.push(mark(
                pre,
                vec![
                    "-A".to_string(),
                    pre.to_string(),
                    "-d".to_string(),
                    cidr.clone(),
                    "-p".to_string(),
                    proto.to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            ));
        }
    }
    rules.extend(user_rules_for(
        document,
        mode,
        family,
        Scope::Forward,
        "mangle",
        pre,
    ));
    if let Some(set) = cnip_set {
        rules.push(mark(
            pre,
            vec![
                "-A".to_string(),
                pre.to_string(),
                "-m".to_string(),
                "set".to_string(),
                "--match-set".to_string(),
                set.to_string(),
                "dst".to_string(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                bypass_mark.to_string(),
            ],
        ));
    }
    rules.push(mark(
        pre,
        vec![
            "-A".to_string(),
            pre.to_string(),
            "-m".to_string(),
            "mark".to_string(),
            "--mark".to_string(),
            bypass_mark.to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    ));
    rules.push(mark(
        pre,
        vec![
            "-A".to_string(),
            pre.to_string(),
            "-j".to_string(),
            "MARK".to_string(),
            "--set-xmark".to_string(),
            route_mark.to_string(),
        ],
    ));
    rules.push(mark(
        out,
        vec![
            "-A".to_string(),
            out.to_string(),
            "-m".to_string(),
            "owner".to_string(),
            "--uid-owner".to_string(),
            "0".to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    ));
    for direction in ["--dport", "--sport"] {
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                direction.to_string(),
                "5555".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    rules.extend(user_rules_for(
        document,
        mode,
        family,
        Scope::Local,
        "mangle",
        out,
    ));
    let proxy_mode = settings
        .get("proxy_mode")
        .and_then(Value::as_str)
        .unwrap_or("core");
    let uids = settings
        .get("__proxy_uids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u32::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let gids = settings
        .get("__proxy_gids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u32::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if proxy_mode == "blacklist" || proxy_mode == "black" {
        for uid in &uids {
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    uid.to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            ));
        }
        for gid in &gids {
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--gid-owner".to_string(),
                    gid.to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            ));
        }
    } else if (proxy_mode == "whitelist" || proxy_mode == "white")
        && (!uids.is_empty() || !gids.is_empty())
    {
        for uid in &uids {
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    uid.to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    route_mark.to_string(),
                ],
            ));
        }
        for gid in &gids {
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--gid-owner".to_string(),
                    gid.to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    route_mark.to_string(),
                ],
            ));
        }
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    rules.push(mark(
        out,
        vec![
            "-A".to_string(),
            out.to_string(),
            "-j".to_string(),
            "MARK".to_string(),
            "--set-xmark".to_string(),
            route_mark.to_string(),
        ],
    ));
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            "-4".to_string(),
            "rule".to_string(),
            "add".to_string(),
            "fwmark".to_string(),
            bypass_mark.to_string(),
            "table".to_string(),
            "main".to_string(),
            "pref".to_string(),
            "98".to_string(),
        ],
    });
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            "-4".to_string(),
            "rule".to_string(),
            "add".to_string(),
            "fwmark".to_string(),
            route_mark.to_string(),
            "table".to_string(),
            "2025".to_string(),
            "pref".to_string(),
            "99".to_string(),
        ],
    });
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            "-4".to_string(),
            "route".to_string(),
            "replace".to_string(),
            "default".to_string(),
            "dev".to_string(),
            tun_device.to_string(),
            "table".to_string(),
            "2025".to_string(),
        ],
    });
    rules
}

fn quic_block_rules(settings: &Value) -> Vec<Rule> {
    if !settings
        .get("quic")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "disable")
    {
        return Vec::new();
    }
    let mut rules = vec![
        Rule {
            family: "iptables",
            table: "filter",
            chain: "BOX_QUIC_BLOCK",
            args: vec!["-N".to_string(), "BOX_QUIC_BLOCK".to_string()],
        },
        Rule {
            family: "iptables",
            table: "filter",
            chain: "OUTPUT",
            args: vec![
                "-A".to_string(),
                "OUTPUT".to_string(),
                "-j".to_string(),
                "BOX_QUIC_BLOCK".to_string(),
            ],
        },
        Rule {
            family: "iptables",
            table: "filter",
            chain: "FORWARD",
            args: vec![
                "-A".to_string(),
                "FORWARD".to_string(),
                "-j".to_string(),
                "BOX_QUIC_BLOCK".to_string(),
            ],
        },
    ];
    for port in [443, 80] {
        rules.push(Rule {
            family: "iptables",
            table: "filter",
            chain: "BOX_QUIC_BLOCK",
            args: vec![
                "-A".to_string(),
                "BOX_QUIC_BLOCK".to_string(),
                "-p".to_string(),
                "udp".to_string(),
                "--dport".to_string(),
                port.to_string(),
                "-j".to_string(),
                "REJECT".to_string(),
            ],
        });
    }
    rules
}

#[allow(clippy::too_many_arguments)]
fn local_udp_tproxy_rules(
    family: &'static str,
    port: u16,
    proxy_mode: &str,
    uids: &[u32],
    gids: &[u32],
    document: Option<&NetworkRuleDocument>,
    mode: &str,
) -> Vec<Rule> {
    let chain = "BOX_LOCAL_UDP";
    let mut rules = local_core_bypass_rules(family, "mangle", chain);
    rules.extend(user_rules_for(
        document,
        mode,
        family,
        Scope::Local,
        "mangle",
        chain,
    ));
    for direction in ["--dport", "--sport"] {
        rules.push(Rule {
            family,
            table: "mangle",
            chain,
            args: vec![
                "-A".to_string(),
                chain.to_string(),
                "-p".to_string(),
                "udp".to_string(),
                direction.to_string(),
                "5555".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    let mark = |rules: &mut Vec<Rule>, uid: Option<u32>, gid: Option<u32>, target: &str| {
        let mut args = vec!["-A".to_string(), chain.to_string()];
        args.extend(["-m".to_string(), "owner".to_string()]);
        if let Some(uid) = uid {
            args.extend(["--uid-owner".to_string(), uid.to_string()]);
        }
        if let Some(gid) = gid {
            args.extend(["--gid-owner".to_string(), gid.to_string()]);
        }
        args.extend([
            "-p".to_string(),
            "udp".to_string(),
            "-j".to_string(),
            target.to_string(),
        ]);
        if target == "MARK" {
            args.extend([
                "--set-xmark".to_string(),
                "0x01000000/0x01000000".to_string(),
            ]);
        }
        rules.push(Rule {
            family,
            table: "mangle",
            chain,
            args,
        });
    };
    let mark_all = |rules: &mut Vec<Rule>| {
        rules.push(Rule {
            family,
            table: "mangle",
            chain,
            args: vec![
                "-A".to_string(),
                chain.to_string(),
                "-p".to_string(),
                "udp".to_string(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                "0x01000000/0x01000000".to_string(),
            ],
        });
    };
    if proxy_mode == "blacklist" || proxy_mode == "black" {
        for uid in uids {
            mark(&mut rules, Some(*uid), None, "RETURN");
        }
        for gid in gids {
            mark(&mut rules, None, Some(*gid), "RETURN");
        }
        mark_all(&mut rules);
    } else if proxy_mode == "whitelist" || proxy_mode == "white" {
        if uids.is_empty() && gids.is_empty() {
            mark_all(&mut rules);
        } else {
            for uid in uids {
                mark(&mut rules, Some(*uid), None, "MARK");
            }
            for gid in gids {
                mark(&mut rules, None, Some(*gid), "MARK");
            }
        }
    } else {
        mark_all(&mut rules);
    }
    let _ = port;
    rules
}

#[allow(clippy::too_many_arguments)]
fn local_proxy_rules(
    family: &'static str,
    table: &'static str,
    chain: &'static str,
    action: &str,
    port: u16,
    proxy_tcp: bool,
    proxy_udp: bool,
    proxy_mode: &str,
    uids: &[u32],
    gids: &[u32],
    document: Option<&NetworkRuleDocument>,
    mode: &str,
) -> Vec<Rule> {
    let mut rules = local_core_bypass_rules(family, table, chain);
    for direction in ["--dport", "--sport"] {
        rules.push(Rule {
            family,
            table,
            chain,
            args: vec![
                "-A".to_string(),
                chain.to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                direction.to_string(),
                "5555".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    rules.extend(user_rules_for(
        document,
        mode,
        family,
        Scope::Local,
        table,
        chain,
    ));
    let hidden = format!("__local_{action}");
    let _ = hidden;
    let push_owner = |rules: &mut Vec<Rule>,
                      protocol: &str,
                      uid: Option<u32>,
                      gid: Option<u32>,
                      target: &str| {
        let mut args = vec!["-A".to_string(), chain.to_string()];
        args.extend(["-m".to_string(), "owner".to_string()]);
        if let Some(uid) = uid {
            args.extend(["--uid-owner".to_string(), uid.to_string()]);
        }
        if let Some(gid) = gid {
            args.extend(["--gid-owner".to_string(), gid.to_string()]);
        }
        args.extend([
            "-p".to_string(),
            protocol.to_string(),
            "-j".to_string(),
            target.to_string(),
        ]);
        if target == "MARK" {
            args.extend([
                "--set-xmark".to_string(),
                "0x01000000/0x01000000".to_string(),
            ]);
        } else if target == "REDIRECT" {
            args.extend(["--to-ports".to_string(), port.to_string()]);
        }
        rules.push(Rule {
            family,
            table,
            chain,
            args,
        });
    };
    let push_all = |rules: &mut Vec<Rule>, protocol: &str, target: &str| {
        let mut args = vec![
            "-A".to_string(),
            chain.to_string(),
            "-p".to_string(),
            protocol.to_string(),
            "-j".to_string(),
            target.to_string(),
        ];
        if target == "MARK" {
            args.extend([
                "--set-xmark".to_string(),
                "0x01000000/0x01000000".to_string(),
            ]);
        } else if target == "REDIRECT" {
            args.extend(["--to-ports".to_string(), port.to_string()]);
        }
        rules.push(Rule {
            family,
            table,
            chain,
            args,
        });
    };
    if proxy_mode == "blacklist" || proxy_mode == "black" {
        for protocol in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !protocol.1 {
                continue;
            }
            for uid in uids {
                push_owner(&mut rules, protocol.0, Some(*uid), None, "RETURN");
            }
            for gid in gids {
                push_owner(&mut rules, protocol.0, None, Some(*gid), "RETURN");
            }
        }
        if proxy_tcp {
            push_all(&mut rules, "tcp", action_target(action));
        }
        if proxy_udp {
            push_all(&mut rules, "udp", action_target(action));
        }
    } else if proxy_mode == "whitelist" || proxy_mode == "white" {
        if uids.is_empty() && gids.is_empty() {
            if proxy_tcp {
                push_all(&mut rules, "tcp", action_target(action));
            }
            if proxy_udp {
                push_all(&mut rules, "udp", action_target(action));
            }
        } else {
            for protocol in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
                if !protocol.1 {
                    continue;
                }
                for uid in uids {
                    push_owner(
                        &mut rules,
                        protocol.0,
                        Some(*uid),
                        None,
                        action_target(action),
                    );
                }
                for gid in gids {
                    push_owner(
                        &mut rules,
                        protocol.0,
                        None,
                        Some(*gid),
                        action_target(action),
                    );
                }
            }
        }
    } else {
        if proxy_tcp {
            push_all(&mut rules, "tcp", action_target(action));
        }
        if proxy_udp {
            push_all(&mut rules, "udp", action_target(action));
        }
    }
    rules
}

fn action_target(action: &str) -> &'static str {
    if action == "mark" {
        "MARK"
    } else {
        "REDIRECT"
    }
}

#[allow(clippy::too_many_arguments)]
fn tproxy_rules(
    family: &'static str,
    port: u16,
    proxy_tcp: bool,
    proxy_udp: bool,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
    document: Option<&NetworkRuleDocument>,
    mode: &str,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "BOX_TPROXY",
        args: vec!["-N".to_string(), "BOX_TPROXY".to_string()],
    });
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "BOX_TPROXY",
        args: vec![
            "-A".to_string(),
            "BOX_TPROXY".to_string(),
            "-m".to_string(),
            "socket".to_string(),
            "--transparent".to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    });
    rules.extend(local_core_bypass_rules(
        family,
        "mangle",
        "BOX_TPROXY_LOCAL",
    ));
    for cidr in force_cidrs {
        if proxy_tcp {
            rules.push(Rule {
                family,
                table: "mangle",
                chain: "BOX_TPROXY",
                args: vec![
                    "-A".to_string(),
                    "BOX_TPROXY".to_string(),
                    "-d".to_string(),
                    cidr.clone(),
                    "-p".to_string(),
                    "tcp".to_string(),
                    "-j".to_string(),
                    "TPROXY".to_string(),
                    "--on-port".to_string(),
                    port.to_string(),
                    "--tproxy-mark".to_string(),
                    "0x01000000/0x01000000".to_string(),
                ],
            });
        }
        if proxy_udp {
            rules.push(Rule {
                family,
                table: "mangle",
                chain: "BOX_TPROXY",
                args: vec![
                    "-A".to_string(),
                    "BOX_TPROXY".to_string(),
                    "-d".to_string(),
                    cidr.clone(),
                    "-p".to_string(),
                    "udp".to_string(),
                    "-j".to_string(),
                    "TPROXY".to_string(),
                    "--on-port".to_string(),
                    port.to_string(),
                    "--tproxy-mark".to_string(),
                    "0x01000000/0x01000000".to_string(),
                ],
            });
        }
    }
    rules.extend(user_rules_for(
        document,
        mode,
        family,
        Scope::Forward,
        "mangle",
        "BOX_TPROXY",
    ));
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "BOX_TPROXY",
        args: vec![
            "-A".to_string(),
            "BOX_TPROXY".to_string(),
            "-m".to_string(),
            "mark".to_string(),
            "--mark".to_string(),
            "0x01000000/0x01000000".to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    });
    if let Some(set) = cnip_set {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: "BOX_TPROXY",
            args: vec![
                "-A".to_string(),
                "BOX_TPROXY".to_string(),
                "-m".to_string(),
                "set".to_string(),
                "--match-set".to_string(),
                set.to_string(),
                "dst".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    for direction in ["--dport", "--sport"] {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: "BOX_TPROXY",
            args: vec![
                "-A".to_string(),
                "BOX_TPROXY".to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                direction.to_string(),
                "5555".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    if proxy_tcp {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: "BOX_TPROXY",
            args: vec![
                "-A".to_string(),
                "BOX_TPROXY".to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "-j".to_string(),
                "TPROXY".to_string(),
                "--on-port".to_string(),
                port.to_string(),
                "--tproxy-mark".to_string(),
                "0x01000000/0x01000000".to_string(),
            ],
        });
    }
    if proxy_udp {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: "BOX_TPROXY",
            args: vec![
                "-A".to_string(),
                "BOX_TPROXY".to_string(),
                "-p".to_string(),
                "udp".to_string(),
                "-j".to_string(),
                "TPROXY".to_string(),
                "--on-port".to_string(),
                port.to_string(),
                "--tproxy-mark".to_string(),
                "0x01000000/0x01000000".to_string(),
            ],
        });
    }
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "PREROUTING",
        args: vec![
            "-A".to_string(),
            "PREROUTING".to_string(),
            "-j".to_string(),
            "BOX_TPROXY".to_string(),
        ],
    });
    let ip_family = if family == "ip6tables" { "-6" } else { "-4" };
    let route_network = if family == "ip6tables" {
        "::/0"
    } else {
        "0.0.0.0/0"
    };
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            ip_family.to_string(),
            "rule".to_string(),
            "add".to_string(),
            "fwmark".to_string(),
            "0x01000000/0x01000000".to_string(),
            "table".to_string(),
            "100".to_string(),
        ],
    });
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            ip_family.to_string(),
            "route".to_string(),
            "add".to_string(),
            "local".to_string(),
            route_network.to_string(),
            "dev".to_string(),
            "lo".to_string(),
            "table".to_string(),
            "100".to_string(),
        ],
    });
    rules
}

fn redirect_rules(
    family: &'static str,
    port: u16,
    proxy_tcp: bool,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
    document: Option<&NetworkRuleDocument>,
    mode: &str,
) -> Vec<Rule> {
    let mut rules = vec![
        Rule {
            family,
            table: "nat",
            chain: "BOX_REDIRECT",
            args: vec!["-N".to_string(), "BOX_REDIRECT".to_string()],
        },
        Rule {
            family,
            table: "nat",
            chain: "BOX_REDIRECT",
            args: vec![
                "-A".to_string(),
                "BOX_REDIRECT".to_string(),
                "-m".to_string(),
                "socket".to_string(),
                "--transparent".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        },
    ];
    rules.extend(local_core_bypass_rules(family, "nat", "BOX_REDIRECT_LOCAL"));
    for cidr in force_cidrs {
        if proxy_tcp {
            rules.push(Rule {
                family,
                table: "nat",
                chain: "BOX_REDIRECT",
                args: vec![
                    "-A".to_string(),
                    "BOX_REDIRECT".to_string(),
                    "-d".to_string(),
                    cidr.clone(),
                    "-p".to_string(),
                    "tcp".to_string(),
                    "-j".to_string(),
                    "REDIRECT".to_string(),
                    "--to-ports".to_string(),
                    port.to_string(),
                ],
            });
        }
    }
    rules.extend(user_rules_for(
        document,
        mode,
        family,
        Scope::Forward,
        "nat",
        "BOX_REDIRECT",
    ));
    if let Some(set) = cnip_set {
        rules.push(Rule {
            family,
            table: "nat",
            chain: "BOX_REDIRECT",
            args: vec![
                "-A".to_string(),
                "BOX_REDIRECT".to_string(),
                "-m".to_string(),
                "set".to_string(),
                "--match-set".to_string(),
                set.to_string(),
                "dst".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    if proxy_tcp {
        rules.push(Rule {
            family,
            table: "nat",
            chain: "BOX_REDIRECT",
            args: vec![
                "-A".to_string(),
                "BOX_REDIRECT".to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "-j".to_string(),
                "REDIRECT".to_string(),
                "--to-ports".to_string(),
                port.to_string(),
            ],
        });
    }
    rules.push(Rule {
        family,
        table: "nat",
        chain: "PREROUTING",
        args: vec![
            "-A".to_string(),
            "PREROUTING".to_string(),
            "-j".to_string(),
            "BOX_REDIRECT".to_string(),
        ],
    });
    rules
}

fn port(settings: &Value, key: &str) -> Result<u16> {
    let value = settings
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| BoxError::validation(format!("{key} is missing")))?;
    let port = value
        .parse::<u16>()
        .map_err(|_| BoxError::validation(format!("{key} is not a port")))?;
    if port == 0 {
        return Err(BoxError::validation(format!("{key} is out of range")));
    }
    Ok(port)
}

fn cnip_force_proxy_uids(settings: &Value) -> Vec<u32> {
    let apps = settings
        .get("cnip_force_proxy_apps")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if apps.is_empty() {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string("/data/system/packages.list") else {
        return Vec::new();
    };
    parse_force_uids(&apps, &text)
}

fn parse_force_uids(apps: &[&str], packages_list: &str) -> Vec<u32> {
    let mut packages = std::collections::BTreeMap::new();
    for line in packages_list.lines() {
        let mut fields = line.split_whitespace();
        let Some(package) = fields.next() else {
            continue;
        };
        let Some(uid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        packages.insert(package.to_string(), uid);
    }
    let mut uids = std::collections::BTreeSet::new();
    for entry in apps {
        let (user, package) = entry
            .split_once(':')
            .map_or(("0", *entry), |(user, package)| (user, package));
        let user = user.parse::<u32>().unwrap_or(0);
        if let Some(base) = packages.get(package) {
            uids.insert(base.saturating_add(user.saturating_mul(100_000)));
        }
    }
    uids.into_iter().collect()
}

fn string_list(settings: &Value, key: &str) -> Vec<String> {
    settings
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn enabled(settings: &Value, key: &str) -> bool {
    settings
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true")
}

fn has_command(name: &str) -> bool {
    if let Some(path) = env::var_os("PATH") {
        if env::split_paths(&path).any(|dir| dir.join(name).is_file()) {
            return true;
        }
    }
    SYSTEM_PATH
        .iter()
        .any(|dir| Path::new(dir).join(name).is_file())
}

fn has_table(command: &str, table: &str) -> bool {
    if !has_command(command) {
        return false;
    }
    let mut probe = std::process::Command::new(command);
    if matches!(command, "iptables" | "ip6tables") {
        // Capability probes must never wait on the global xtables lock held
        // by a concurrent service transaction.
        probe.args(["-w", "1"]);
    }
    probe
        .args(["-t", table, "-L"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn api_level() -> u32 {
    if !has_command("getprop") {
        return 0;
    }
    std::process::Command::new("getprop")
        .arg("ro.build.version.sdk")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{apply, plan, plan_with_dns_and_rules, Capabilities, Rule, Runner};
    use serde_json::json;

    fn settings(mode: &str) -> serde_json::Value {
        json!({
            "network_mode": mode,
            "tproxy_port": "7893",
            "redir_port": "7892",
            "ipv6": "false",
            "proxy_tcp": "true",
            "proxy_udp": "true",
            "bypass_cn_ip": "false",
            "ipset": "false"
        })
    }

    fn default_rule_document() -> crate::network_rules::Document {
        crate::network_rules::Document {
            schema_version: 1,
            revision: 1,
            defaults_version: "test".to_string(),
            tombstones: Vec::new(),
            rules: vec![crate::network_rules::RuleSpec {
                id: "default-private-v4".to_string(),
                name: "private".to_string(),
                enabled: true,
                action: crate::network_rules::Action::Bypass,
                scopes: vec![
                    crate::network_rules::Scope::Local,
                    crate::network_rules::Scope::Forward,
                ],
                modes: vec![
                    crate::network_rules::Mode::Tun,
                    crate::network_rules::Mode::Tproxy,
                    crate::network_rules::Mode::Redirect,
                    crate::network_rules::Mode::Mixed,
                    crate::network_rules::Mode::Enhance,
                ],
                family: crate::network_rules::Family::Ipv4,
                protocol: crate::network_rules::Protocol::Any,
                match_fields: crate::network_rules::MatchFields {
                    dst_cidrs: vec!["10.0.0.0/8".to_string()],
                    ..Default::default()
                },
                origin: crate::network_rules::Origin::Default,
            }],
        }
    }

    struct FakeRunner {
        commands: Vec<Vec<String>>,
        fail_at: Option<usize>,
    }

    impl Runner for FakeRunner {
        fn run(&mut self, command: &str, args: &[String]) -> crate::error::Result<()> {
            self.commands.push(
                std::iter::once(command.to_string())
                    .chain(args.iter().cloned())
                    .collect(),
            );
            if self.fail_at == Some(self.commands.len()) {
                return Err(crate::error::BoxError::new(
                    crate::error::ErrorCode::Command,
                    "injected failure",
                ));
            }
            Ok(())
        }
    }

    fn full() -> Capabilities {
        Capabilities {
            iptables: true,
            ip6tables: true,
            ip6tables_nat: true,
            ip6tables_mangle: true,
            ipset: true,
            nft: false,
            tun: true,
            api_level: 35,
        }
    }

    #[test]
    fn plan_validation_rejects_duplicate_chain_creation() {
        let plan = super::Plan {
            requested_mode: "tproxy".to_string(),
            effective_mode: "tproxy".to_string(),
            degraded: false,
            rules: vec![
                super::Rule {
                    family: "iptables",
                    table: "mangle",
                    chain: "BOX_DUP",
                    args: vec!["-N".to_string(), "BOX_DUP".to_string()],
                },
                super::Rule {
                    family: "iptables",
                    table: "mangle",
                    chain: "BOX_DUP",
                    args: vec!["-N".to_string(), "BOX_DUP".to_string()],
                },
            ],
            warnings: Vec::new(),
        };
        assert!(plan.validate().is_err());
    }

    #[test]
    fn reject_rules_compile_into_filter_chains() {
        let mut document = default_rule_document();
        document.rules.push(crate::network_rules::RuleSpec {
            id: "user-reject".to_string(),
            name: "reject".to_string(),
            enabled: true,
            action: crate::network_rules::Action::Reject,
            scopes: vec![crate::network_rules::Scope::Local],
            modes: vec![crate::network_rules::Mode::Tun],
            family: crate::network_rules::Family::Ipv4,
            protocol: crate::network_rules::Protocol::Tcp,
            match_fields: crate::network_rules::MatchFields {
                dst_cidrs: vec!["198.51.100.77/32".to_string()],
                ..Default::default()
            },
            origin: crate::network_rules::Origin::User,
        });
        let values = settings("tun");
        let plan = plan_with_dns_and_rules(&values, full(), Some(1053), Some(&document))
            .expect("tun filter plan");
        assert!(plan.rules.iter().any(|rule| {
            rule.family == "iptables"
                && rule.table == "filter"
                && rule.chain == "BOX_FILTER_LOCAL"
                && rule.args.iter().any(|arg| arg == "REJECT")
        }));
    }

    #[test]
    fn packaged_default_rules_compile_into_bypass_chains() {
        let bytes = include_str!("../../../../box/network-rules.default.json");
        let document: crate::network_rules::Document =
            serde_json::from_str(bytes).expect("default rules");
        let mut values = settings("tproxy");
        values["ipv6"] = json!("false");
        let plan =
            plan_with_dns_and_rules(&values, full(), Some(1053), Some(&document)).expect("plan");
        assert!(plan.rules.iter().any(|rule| {
            rule.chain == "BOX_LOCAL_REDIRECT"
                && rule.args.iter().any(|arg| arg == "192.168.0.0/16")
                && rule.args.iter().any(|arg| arg == "RETURN")
        }));
        assert!(plan.rules.iter().any(|rule| {
            rule.chain == "BOX_TPROXY"
                && rule.args.iter().any(|arg| arg == "5353")
                && rule.args.iter().any(|arg| arg == "RETURN")
        }));
    }

    #[test]
    fn local_udp_uses_tproxy_mark_in_tproxy_mode() {
        let values = settings("tproxy");
        let plan = plan(&values, full()).expect("tproxy plan");
        assert!(plan.rules.iter().any(|rule| {
            rule.table == "mangle"
                && rule.chain == "BOX_LOCAL_UDP"
                && rule.args.iter().any(|arg| arg == "MARK")
        }));
    }

    #[test]
    fn quic_disable_adds_filter_reject_rules() {
        let mut values = settings("tproxy");
        values["quic"] = json!("disable");
        let plan = plan(&values, full()).expect("quic plan");
        assert!(plan.rules.iter().any(|rule| {
            rule.table == "filter"
                && rule.chain == "BOX_QUIC_BLOCK"
                && rule.args.iter().any(|arg| arg == "443")
                && rule.args.iter().any(|arg| arg == "REJECT")
        }));
        assert!(plan.rules.iter().any(|rule| {
            rule.table == "filter"
                && rule.chain == "FORWARD"
                && rule.args.iter().any(|arg| arg == "BOX_QUIC_BLOCK")
        }));
    }

    #[test]
    fn local_output_rules_follow_proxy_mode() {
        let mut core = settings("tproxy");
        core["proxy_mode"] = serde_json::json!("core");
        let core_plan = plan(&core, full()).expect("core");
        assert!(core_plan.rules.iter().any(|rule| {
            rule.chain == "BOX_LOCAL_REDIRECT" && rule.args.iter().any(|arg| arg == "REDIRECT")
        }));

        let mut black = settings("tproxy");
        black["proxy_mode"] = serde_json::json!("blacklist");
        black["__proxy_uids"] = serde_json::json!([10001]);
        let black_plan = plan(&black, full()).expect("blacklist");
        assert!(black_plan.rules.iter().any(|rule| {
            rule.chain == "BOX_LOCAL_REDIRECT"
                && rule.args.iter().any(|arg| arg == "10001")
                && rule.args.iter().any(|arg| arg == "RETURN")
        }));
        assert!(black_plan.rules.iter().any(|rule| {
            rule.chain == "BOX_LOCAL_REDIRECT" && rule.args.iter().any(|arg| arg == "REDIRECT")
        }));

        let mut white = settings("tproxy");
        white["proxy_mode"] = serde_json::json!("whitelist");
        white["__proxy_uids"] = serde_json::json!([10002]);
        let white_plan = plan(&white, full()).expect("whitelist");
        assert!(white_plan.rules.iter().any(|rule| {
            rule.chain == "BOX_LOCAL_REDIRECT"
                && rule.args.iter().any(|arg| arg == "10002")
                && rule.args.iter().any(|arg| arg == "REDIRECT")
        }));
    }

    #[test]
    fn enhance_plan_has_no_duplicate_chain_creation() {
        let plan = plan(&settings("enhance"), full()).expect("enhance plan");
        plan.validate()
            .expect("enhance plan must be structurally valid");
        assert!(plan.rules.iter().any(|rule| rule.chain == "BOX_TPROXY"));
        assert!(plan.rules.iter().any(|rule| rule.chain == "BOX_REDIRECT"));
    }

    #[test]
    fn tun_has_bypass_rules() {
        let plan = plan(&settings("tun"), full()).expect("plan");
        assert!(plan
            .rules
            .iter()
            .any(|rule| rule.chain == "BOX_TUN_BYPASS_PRE"));
        assert!(plan
            .rules
            .iter()
            .any(|rule| rule.chain == "BOX_TUN_BYPASS_OUT"));
        assert!(!plan.degraded);
    }

    #[test]
    fn tproxy_and_redirect_have_rules() {
        let tproxy = plan(&settings("tproxy"), full()).expect("tproxy");
        assert!(!tproxy.rules.is_empty());
        assert!(tproxy
            .rules
            .iter()
            .any(|rule| rule.family == "ip" && rule.args.iter().any(|arg| arg == "rule")));
        let redirect = plan(&settings("redirect"), full()).expect("redirect");
        assert!(!redirect.rules.is_empty());
        let mixed = plan(&settings("mixed"), full()).expect("mixed");
        assert!(mixed.rules.iter().any(|rule| rule.chain == "BOX_REDIRECT"));
        assert!(mixed
            .rules
            .iter()
            .any(|rule| rule.chain == "BOX_TUN_BYPASS_PRE"));
        assert!(!mixed.rules.iter().any(|rule| rule.chain == "BOX_TPROXY"));
    }

    #[test]
    fn non_tun_modes_emit_dns_hijack_rules() {
        for mode in ["tproxy", "redirect", "mixed", "enhance"] {
            let mut values = settings(mode);
            values["dns_hijack_udp"] = serde_json::json!("true");
            let plan = super::plan_with_dns(&values, full(), Some(1053)).expect("dns plan");
            assert!(
                plan.rules.iter().any(|rule| rule.chain == "BOX_DNS"),
                "{mode} missing DNS chain"
            );
            assert!(
                plan.rules
                    .iter()
                    .any(|rule| rule.args.iter().any(|arg| arg == "1053")),
                "{mode} missing DNS port"
            );
        }
    }

    #[test]
    fn tun_dns_hijack_rules_are_generated() {
        let mut values = settings("tun");
        values["dns_hijack_udp"] = serde_json::json!("true");
        values["dns_hijack_tcp"] = serde_json::json!("true");
        let plan = super::plan_with_dns(&values, full(), Some(1053)).expect("tun dns");
        assert!(plan
            .rules
            .iter()
            .any(|rule| rule.chain == "BOX_DNS" && rule.table == "nat"));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "1053")
                && rule.args.iter().any(|arg| arg == "REDIRECT")
        }));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "OUTPUT")
                && rule.args.iter().any(|arg| arg == "BOX_DNS")
        }));
    }

    #[test]
    fn nft_backend_generates_table_and_cleanup() {
        let mut caps = full();
        caps.iptables = false;
        caps.nft = true;
        let plan = plan(&settings("tproxy"), caps).expect("nft plan");
        assert!(!plan.rules.is_empty());
        assert_eq!(plan.rules[0].family, "nft");
        assert!(plan.rules[0].args.iter().any(|arg| arg == "table"));
        let chain = plan
            .rules
            .iter()
            .find(|rule| rule.args.iter().any(|arg| arg == "chain"))
            .expect("nft chain");
        assert!(chain.args.iter().any(|arg| arg == "box_tproxy"));
        assert!(chain.args.iter().any(|arg| arg.contains("hook prerouting")));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "box_tproxy")
                && rule.args.iter().any(|arg| arg == "tproxy")
        }));
        let mut runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        apply(&plan, &mut runner).expect("apply");
        assert!(runner
            .commands
            .iter()
            .any(|command| command.iter().any(|arg| arg == "nft")));
        let mut cleanup_runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        super::cleanup(&plan, &mut cleanup_runner).expect("cleanup");
        assert!(cleanup_runner
            .commands
            .iter()
            .any(|command| command.windows(2).any(|pair| pair == ["delete", "table"])));
    }

    #[test]
    fn tun_nft_dns_hijack_rules_are_generated() {
        let mut caps = full();
        caps.iptables = false;
        caps.nft = true;
        let mut values = settings("tun");
        values["dns_hijack_udp"] = serde_json::json!("true");
        values["dns_hijack_tcp"] = serde_json::json!("true");
        let plan = super::plan_with_dns(&values, caps, Some(1053)).expect("tun nft dns");
        assert!(plan
            .rules
            .iter()
            .any(|rule| rule.args.iter().any(|arg| arg == "box_dns")));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "redirect")
                && rule.args.iter().any(|arg| arg == ":1053")
        }));
    }

    #[test]
    fn nft_ipv6_tproxy_rules_are_generated() {
        let mut caps = full();
        caps.iptables = false;
        caps.nft = true;
        let mut values = settings("tproxy");
        values["ipv6"] = serde_json::json!("true");
        let plan = plan(&values, caps).expect("nft ipv6");
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "ipv6")
                && rule.args.iter().any(|arg| arg == "[::]:7893")
        }));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "ipv4") && rule.args.iter().any(|arg| arg == "tproxy")
        }));
    }

    #[test]
    fn nft_dns_hijack_rules_are_generated() {
        let mut caps = full();
        caps.iptables = false;
        caps.nft = true;
        let mut values = settings("tproxy");
        values["dns_hijack_udp"] = serde_json::json!("true");
        values["dns_hijack_tcp"] = serde_json::json!("true");
        let plan = super::plan_with_dns(&values, caps, Some(1053)).expect("nft dns");
        assert!(plan
            .rules
            .iter()
            .any(|rule| rule.args.iter().any(|arg| arg == "box_dns")));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "53")
                && rule.args.iter().any(|arg| arg == "redirect")
                && rule.args.iter().any(|arg| arg == ":1053")
        }));
    }

    #[test]
    fn nft_modes() {
        let mut caps = full();
        caps.iptables = false;
        caps.nft = true;
        caps.api_level = 31;
        let mixed = plan(&settings("mixed"), caps).expect("nft mixed");
        assert!(mixed
            .rules
            .iter()
            .any(|rule| rule.family == "nft" && rule.args.iter().any(|arg| arg == "tproxy")));
        assert!(mixed
            .rules
            .iter()
            .any(|rule| rule.family == "nft" && rule.args.iter().any(|arg| arg == "redirect")));
    }

    #[test]
    fn cnip_force_proxy_apps_use_output_local_chains() {
        let tproxy =
            super::tproxy_local_force_rules("iptables", "cnip", &[10080], 9898, true, true);
        assert!(tproxy.iter().any(|rule| {
            rule.chain == "OUTPUT" && rule.args.iter().any(|arg| arg == "BOX_LOCAL_TPROXY")
        }));
        assert!(tproxy.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "--uid-owner")
                && rule.args.iter().any(|arg| arg == "10080")
                && rule.args.iter().any(|arg| arg == "MARK")
        }));
        let redirect = super::redirect_local_force_rules("iptables", "cnip", &[10080], 9797, true);
        assert!(redirect.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "--uid-owner")
                && rule.args.iter().any(|arg| arg == "REDIRECT")
                && rule.args.iter().any(|arg| arg == "9797")
        }));
    }

    #[test]
    fn cnip_force_proxy_uids_handle_multiuser_prefix() {
        let list = "com.example.app 10123 0 /data/user/0/com.example.app\n";
        assert_eq!(
            super::parse_force_uids(&["com.example.app", "10:com.example.app"], list),
            vec![10123, 1_010_123]
        );
        assert!(super::parse_force_uids(&["missing.app"], list).is_empty());
    }

    #[test]
    fn force_proxy_cidrs_precede_local_bypass() {
        let mut values = settings("tproxy");
        values["force_proxy_cidrs"] = serde_json::json!(["203.0.113.0/24"]);
        let rules = default_rule_document();
        let plan =
            plan_with_dns_and_rules(&values, full(), None, Some(&rules)).expect("force cidrs");
        let tproxy = plan
            .rules
            .iter()
            .position(|rule| {
                rule.chain == "BOX_TPROXY"
                    && rule.args.iter().any(|arg| arg == "203.0.113.0/24")
                    && rule.args.iter().any(|arg| arg == "TPROXY")
            })
            .expect("tproxy force rule");
        let local = plan
            .rules
            .iter()
            .position(|rule| {
                rule.chain == "BOX_TPROXY" && rule.args.iter().any(|arg| arg == "10.0.0.0/8")
            })
            .expect("local bypass");
        assert!(tproxy < local);
    }

    #[test]
    fn cnip_ipset_bypass_rules_are_generated() {
        let mut caps = full();
        caps.ipset = true;
        let mut values = settings("tproxy");
        values["bypass_cn_ip"] = serde_json::json!("true");
        values["bypass_cn_ip_v4"] = serde_json::json!("true");
        values["cnip_mode"] = serde_json::json!("ipset");
        let plan = plan(&values, caps).expect("cnip plan");
        assert!(plan.rules.iter().any(|rule| {
            rule.family == "ipset"
                && rule.args.iter().any(|arg| arg == "create")
                && rule.args.iter().any(|arg| arg == "cnip")
        }));
        assert!(plan.rules.iter().any(|rule| {
            rule.args.iter().any(|arg| arg == "--match-set")
                && rule.args.iter().any(|arg| arg == "cnip")
        }));
        let bypass_count = plan
            .rules
            .iter()
            .filter(|rule| rule.args.iter().any(|arg| arg == "--match-set"))
            .count();
        assert!(bypass_count >= 1, "CNIP bypass rule must be present");
    }

    #[test]
    fn missing_iptables_and_ipset_degrade() {
        let mut caps = full();
        caps.iptables = false;
        caps.ipset = false;
        let mut values = settings("tproxy");
        values["bypass_cn_ip"] = serde_json::json!("true");
        let plan = plan(&values, caps).expect("plan");
        assert!(plan.degraded);
        assert!(plan.rules.is_empty());
        assert!(plan.warnings.len() >= 2);
    }

    #[test]
    fn apply_runs_plan_and_rollback_shape() {
        let plan = plan(&settings("tproxy"), full()).expect("plan");
        let mut runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        apply(&plan, &mut runner).expect("apply");
        assert_eq!(runner.commands.len(), plan.rules.len());
        assert!(runner
            .commands
            .iter()
            .any(|command| { command.windows(2).any(|pair| pair == ["-t", "mangle"]) }));
        let mut failing = FakeRunner {
            commands: Vec::new(),
            fail_at: Some(2),
        };
        let error = apply(&plan, &mut failing).expect_err("failure");
        assert_eq!(error.code, crate::error::ErrorCode::Command);
        assert!(failing
            .commands
            .iter()
            .any(|command| command.iter().any(|arg| arg == "-F")));
        assert!(failing
            .commands
            .iter()
            .any(|command| command.iter().any(|arg| arg == "-X")));
    }

    #[test]
    fn rollback_removes_ip_rule_and_route() {
        let plan = plan(&settings("tproxy"), full()).expect("plan");
        let mut runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        apply(&plan, &mut runner).expect("apply");
        let mut cleanup_runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        super::cleanup(&plan, &mut cleanup_runner).expect("cleanup");
        assert!(cleanup_runner.commands.iter().any(|command| {
            command.iter().any(|arg| arg == "-4")
                && command.windows(2).any(|pair| pair == ["rule", "del"])
        }));
        assert!(cleanup_runner
            .commands
            .iter()
            .any(|command| { command.windows(2).any(|pair| pair == ["route", "del"]) }));
    }

    #[test]
    fn cleanup_tolerates_already_absent_resources() {
        let absent = crate::error::BoxError::new(
            crate::error::ErrorCode::Command,
            "network rule command failed",
        )
        .with_detail("iptables: No chain/target/match by that name.");
        assert!(super::already_absent(&absent));
        let denied = crate::error::BoxError::new(
            crate::error::ErrorCode::Command,
            "network rule command failed",
        )
        .with_detail("Operation not permitted");
        assert!(!super::already_absent(&denied));
    }

    #[test]
    fn rollback_keeps_rule_arguments() {
        let rule = Rule {
            family: "iptables",
            table: "mangle",
            chain: "BOX_TPROXY",
            args: vec!["-A".to_string(), "BOX_TPROXY".to_string(), "-p".to_string()],
        };
        let rollback = super::rollback_rules(std::iter::once(&rule));
        assert_eq!(rollback.len(), 1);
        assert_eq!(rollback[0].args[0], "-D");
        assert_eq!(rollback[0].args[2], "-p");
    }
}
