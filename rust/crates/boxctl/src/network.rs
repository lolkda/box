use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::network_rules::{Action, Document as NetworkRuleDocument, Family, Mode, Scope};
use crate::paths::Paths;

const SYSTEM_PATH: &[&str] = &["/system/bin", "/system/xbin", "/sbin"];
const TPROXY_MARK: &str = "0x01000000/0x01000000";
const TPROXY_TABLE: &str = "2024";
const TPROXY_PREF: &str = "100";
const TUN_BYPASS_MARK: &str = "0x2000000/0x2000000";
const TUN_ROUTE_MARK: &str = "0x3000000/0x3000000";
const TUN_ROUTE_TABLE: &str = "2025";
const CORE_UID: &str = "0";
const CORE_GID: &str = "3005";
const ANDROID_DNS_UID: u32 = 1052;

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
    pub tproxy_v4: bool,
    pub tproxy_v6: bool,
    pub socket_transparent: bool,
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
            "api_level": self.api_level,
            "tproxy_v4": self.tproxy_v4,
            "tproxy_v6": self.tproxy_v6,
            "socket_transparent": self.socket_transparent
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
        let mut chains: BTreeMap<(&'static str, &'static str, &'static str), usize> =
            BTreeMap::new();
        for rule in &self.rules {
            if !matches!(rule.family, "iptables" | "ip6tables") || rule.table.is_empty() {
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
            if !matches!(rule.family, "iptables" | "ip6tables") || rule.table.is_empty() {
                continue;
            }
            let action = rule.args.first().map(String::as_str).unwrap_or_default();
            if action == "-A" || action == "-I" {
                let parent = rule.args.get(1).map(String::as_str).unwrap_or_default();
                let is_builtin = is_builtin_chain(rule.table, parent);
                if !is_builtin && !chains.contains_key(&(rule.family, rule.table, parent)) {
                    return Err(BoxError::internal(format!(
                        "network plan appends to undefined or invalid chain {}/{}/{}",
                        rule.family, rule.table, parent
                    )));
                }
            }
            if let Some(index) = rule.args.iter().position(|value| value == "-j") {
                if let Some(target) = rule.args.get(index + 1) {
                    if (target.starts_with("BOX_")
                        || target.starts_with("LOCAL_IP_")
                        || target == "DIVERT")
                        && !chains.contains_key(&(rule.family, rule.table, target.as_str()))
                    {
                        return Err(BoxError::internal(format!(
                            "network plan jumps to undefined chain {}/{}/{}",
                            rule.family, rule.table, target
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn is_builtin_chain(table: &str, chain: &str) -> bool {
    match table {
        "filter" => matches!(chain, "INPUT" | "FORWARD" | "OUTPUT"),
        "nat" => matches!(chain, "PREROUTING" | "INPUT" | "OUTPUT" | "POSTROUTING"),
        "mangle" => matches!(
            chain,
            "PREROUTING" | "INPUT" | "FORWARD" | "OUTPUT" | "POSTROUTING"
        ),
        "raw" => matches!(chain, "PREROUTING" | "OUTPUT"),
        _ => false,
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
    rules_doc: Option<&NetworkRuleDocument>,
    runner: &mut dyn Runner,
) -> Result<bool> {
    let mut rules = Vec::new();
    for mode in ["tun", "tproxy", "redirect", "mixed", "enhance"] {
        let mut values = settings.clone();
        values["network_mode"] = Value::String(mode.to_string());
        if let Ok(plan) = plan_with_dns_and_rules(&values, caps, dns_port, rules_doc) {
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
        "BOX_FILTER_FWD",
        "BOX_FILTER_PRE",
        "BOX_LOCAL_UDP",
        "BOX_QUIC_BLOCK",
        "LOCAL_IP_V4",
        "LOCAL_IP_V6",
        "DIVERT",
        // Legacy chain names from box.iptables so upgrades leave zero leftovers.
        "BOX_EXTERNAL",
        "BOX_LOCAL",
        "NAT_DNS_HIJACK",
        "NAT_DNS_HIJACK6",
        "NAT_DNS_FORWARD",
        "NAT_DNS_FORWARD6",
        "BOX_TUN_BYPASS6_PRE",
        "BOX_TUN_BYPASS6_OUT",
    ];
    for family in ["iptables", "ip6tables"] {
        let table_parents: &[(&str, &[&str])] = &[
            ("mangle", &["PREROUTING", "OUTPUT"]),
            ("nat", &["PREROUTING", "OUTPUT"]),
            ("filter", &["INPUT", "FORWARD", "OUTPUT"]),
        ];
        for (table, parents) in table_parents {
            // Remove all parent jumps BEFORE flushing/deleting chains so chains like
            // BOX_QUIC_BLOCK (referenced by both OUTPUT and FORWARD) always delete cleanly.
            for chain in chains {
                for parent in *parents {
                    for _ in 0..4 {
                        if runner
                            .run(
                                family,
                                &[
                                    "-w".to_string(),
                                    "10".to_string(),
                                    "-t".to_string(),
                                    (*table).to_string(),
                                    "-D".to_string(),
                                    (*parent).to_string(),
                                    "-j".to_string(),
                                    chain.to_string(),
                                ],
                            )
                            .is_ok()
                        {
                            touched = true;
                        } else {
                            break;
                        }
                    }
                }
            }
            for chain in chains {
                if runner
                    .run(
                        family,
                        &[
                            "-w".to_string(),
                            "10".to_string(),
                            "-t".to_string(),
                            (*table).to_string(),
                            "-F".to_string(),
                            chain.to_string(),
                        ],
                    )
                    .is_ok()
                {
                    touched = true;
                }
                if runner
                    .run(
                        family,
                        &[
                            "-w".to_string(),
                            "10".to_string(),
                            "-t".to_string(),
                            (*table).to_string(),
                            "-X".to_string(),
                            chain.to_string(),
                        ],
                    )
                    .is_ok()
                {
                    touched = true;
                }
            }
        }
    }
    // Remove nft table if present.
    if runner
        .run(
            "nft",
            &[
                "delete".to_string(),
                "table".to_string(),
                "inet".to_string(),
                "box".to_string(),
            ],
        )
        .is_ok()
    {
        touched = true;
    }
    for ip_flag in ["-4", "-6"] {
        for mark in [TPROXY_MARK, TUN_BYPASS_MARK, TUN_ROUTE_MARK] {
            for _ in 0..4 {
                if runner
                    .run(
                        "ip",
                        &[
                            ip_flag.to_string(),
                            "rule".to_string(),
                            "del".to_string(),
                            "fwmark".to_string(),
                            mark.to_string(),
                        ],
                    )
                    .is_ok()
                {
                    touched = true;
                } else {
                    break;
                }
            }
        }
        for table in ["100", TPROXY_TABLE, TUN_ROUTE_TABLE] {
            if runner
                .run(
                    "ip",
                    &[
                        ip_flag.to_string(),
                        "route".to_string(),
                        "flush".to_string(),
                        "table".to_string(),
                        table.to_string(),
                    ],
                )
                .is_ok()
            {
                touched = true;
            }
        }
    }
    touched
}

pub fn apply_tun_sysctl(tun_device: &str, runner: &mut dyn Runner) {
    let entries = [
        ("net.ipv4.ip_forward", "1"),
        ("net.ipv4.conf.all.rp_filter", "2"),
        ("net.ipv4.conf.default.rp_filter", "2"),
    ];
    for (key, value) in entries {
        let _ = runner.run("sysctl", &["-w".to_string(), format!("{key}={value}")]);
    }
    if !tun_device.is_empty() {
        let _ = runner.run(
            "sysctl",
            &[
                "-w".to_string(),
                format!("net.ipv4.conf.{tun_device}.rp_filter=2"),
            ],
        );
    }
}

pub fn disable_ipv6_policy(runner: &mut dyn Runner) {
    let _ = runner.run("sysctl", &["-w".to_string(), "net.ipv4.ip_forward=1".to_string()]);
    let _ = runner.run(
        "sysctl",
        &[
            "-w".to_string(),
            "net.ipv6.conf.all.forwarding=0".to_string(),
        ],
    );
    set_ipv6_conf_all("accept_ra", "0");
    set_ipv6_conf_all("disable_ipv6", "1");
    let _ = runner.run(
        "ip",
        &[
            "-6".to_string(),
            "rule".to_string(),
            "add".to_string(),
            "unreachable".to_string(),
            "pref".to_string(),
            TPROXY_PREF.to_string(),
        ],
    );
}

pub fn restore_ipv6_policy(runner: &mut dyn Runner) {
    let _ = runner.run("sysctl", &["-w".to_string(), "net.ipv4.ip_forward=1".to_string()]);
    let _ = runner.run(
        "sysctl",
        &[
            "-w".to_string(),
            "net.ipv6.conf.all.forwarding=1".to_string(),
        ],
    );
    set_ipv6_conf_all("disable_ipv6", "0");
    set_ipv6_conf_all("autoconf", "1");
    set_ipv6_conf_all("accept_ra", "2");
    set_ipv6_conf_all("accept_ra_defrtr", "1");
    set_ipv6_conf_all("accept_ra_pinfo", "1");
    let _ = runner.run(
        "ip",
        &[
            "-6".to_string(),
            "rule".to_string(),
            "del".to_string(),
            "unreachable".to_string(),
            "pref".to_string(),
            TPROXY_PREF.to_string(),
        ],
    );
}

fn set_ipv6_conf_all(attr: &str, value: &str) {
    let Ok(entries) = fs::read_dir("/proc/sys/net/ipv6/conf") else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path().join(attr);
        if path.is_file() {
            let _ = fs::write(&path, value);
        }
    }
}

pub fn collect_dynamic_local_ips(tun_device: &str) -> (Vec<String>, Vec<String>) {
    let v4 = std::process::Command::new("ip")
        .args(["-4", "addr", "show", "up"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_ip_addr_output(&String::from_utf8_lossy(&o.stdout), tun_device, false))
        .unwrap_or_default();
    let v6 = std::process::Command::new("ip")
        .args(["-6", "addr", "show", "up"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_ip_addr_output(&String::from_utf8_lossy(&o.stdout), tun_device, true))
        .unwrap_or_default();
    (v4, v6)
}

pub fn parse_ip_addr_output(text: &str, tun_device: &str, ipv6: bool) -> Vec<String> {
    let mut iface = String::new();
    let mut addrs = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !line.starts_with(' ') && !line.starts_with('\t') {
            if let Some(rest) = trimmed.split_once(": ").map(|(_, r)| r) {
                let raw = rest.split(':').next().unwrap_or_default();
                let clean = raw.split('@').next().unwrap_or_default();
                iface = clean.trim().to_string();
            }
            continue;
        }
        if iface.is_empty() || iface == "lo" || iface == tun_device {
            continue;
        }
        if ipv6 {
            if !trimmed.starts_with("inet6 ") || !trimmed.contains("scope global") {
                continue;
            }
            if let Some(cidr) = trimmed.split_whitespace().nth(1) {
                if let Some(addr) = cidr.split('/').next() {
                    addrs.insert(format!("{addr}/128"));
                }
            }
        } else {
            if !trimmed.starts_with("inet ") {
                continue;
            }
            if let Some(cidr) = trimmed.split_whitespace().nth(1) {
                addrs.insert(cidr.to_string());
            }
        }
    }
    addrs.into_iter().collect()
}

pub fn refresh_local_ip_chains(paths: &Paths, runner: &mut dyn Runner) -> Result<(usize, usize)> {
    let tun_dev = crate::service::tun_device_name(paths);
    let (v4, v6) = collect_dynamic_local_ips(&tun_dev);
    refresh_local_ip_chain_family("iptables", "LOCAL_IP_V4", &v4, runner);
    refresh_local_ip_chain_family("ip6tables", "LOCAL_IP_V6", &v6, runner);
    Ok((v4.len(), v6.len()))
}

fn refresh_local_ip_chain_family(
    family: &str,
    chain: &str,
    addrs: &[String],
    runner: &mut dyn Runner,
) {
    for table in ["mangle", "nat"] {
        // Flush only if the chain exists in this table; skip tables that don't have it.
        let flush_args = vec![
            "-w".to_string(),
            "10".to_string(),
            "-t".to_string(),
            table.to_string(),
            "-F".to_string(),
            chain.to_string(),
        ];
        if runner.run(family, &flush_args).is_err() {
            continue;
        }
        for cidr in addrs {
            let _ = runner.run(
                family,
                &[
                    "-w".to_string(),
                    "10".to_string(),
                    "-t".to_string(),
                    table.to_string(),
                    "-A".to_string(),
                    chain.to_string(),
                    "-d".to_string(),
                    cidr.clone(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            );
        }
    }
}

pub fn apply_bond(profile: u8) -> Result<Value> {
    let (low_latency, wlan_qlen, rmnet_qlen, rmnet_mtu) = match profile {
        0 => ("0", "3000", "1000", "1500"),
        1 => ("1", "4000", "2000", "9000"),
        _ => return Err(BoxError::usage("bond profile must be 0 or 1")),
    };
    let mut runner = SystemRunner;
    let _ = runner.run(
        "sysctl",
        &[
            "-w".to_string(),
            format!("net.ipv4.tcp_low_latency={low_latency}"),
        ],
    );
    let mut updated = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with("wlan") {
                if runner
                    .run(
                        "ip",
                        &[
                            "link".to_string(),
                            "set".to_string(),
                            "dev".to_string(),
                            name.clone(),
                            "txqueuelen".to_string(),
                            wlan_qlen.to_string(),
                        ],
                    )
                    .is_ok()
                {
                    updated.push(name);
                }
            } else if name.starts_with("rmnet_data") {
                let q_ok = runner
                    .run(
                        "ip",
                        &[
                            "link".to_string(),
                            "set".to_string(),
                            "dev".to_string(),
                            name.clone(),
                            "txqueuelen".to_string(),
                            rmnet_qlen.to_string(),
                        ],
                    )
                    .is_ok();
                let m_ok = runner
                    .run(
                        "ip",
                        &[
                            "link".to_string(),
                            "set".to_string(),
                            "dev".to_string(),
                            name.clone(),
                            "mtu".to_string(),
                            rmnet_mtu.to_string(),
                        ],
                    )
                    .is_ok();
                if q_ok || m_ok {
                    updated.push(name);
                }
            }
        }
    }
    Ok(json!({
        "profile": format!("bond{profile}"),
        "tcp_low_latency": low_latency,
        "interfaces": updated
    }))
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
            "rule" | "route"
                if rule
                    .args
                    .get(1)
                    .is_some_and(|value| matches!(value.as_str(), "add" | "replace")) =>
            {
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
                    && rule
                        .args
                        .get(2)
                        .is_some_and(|value| matches!(value.as_str(), "add" | "replace")) =>
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
            "description": "精确放行 root:net_admin (0:3005) 核心流量，并通过 loopback 端口拒绝防环路",
            "scopes": ["local"],
            "readOnly": true
        }),
        serde_json::json!({
            "id": "system-local-destination",
            "name": "本机与局域网目标",
            "description": "由默认 CIDR 规则与动态 LOCAL_IP_V4/6 链覆盖，网络切换自动刷新",
            "scopes": ["local", "forward"],
            "readOnly": true
        }),
        serde_json::json!({
            "id": "system-dns-chains",
            "name": "DNS 链结构",
            "description": "BOX_DNS 处理本机 DNS（先于常规重定向），BOX_DNS_PRE 处理转发 DNS",
            "scopes": ["dns"],
            "readOnly": true
        }),
        serde_json::json!({
            "id": "system-ip-rules",
            "name": "策略路由",
            "description": "tproxy (table 2024 pref 100) 与 TUN CNIP (table 2025 pref 98/99) 由模块固定管理",
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
    let iptables = has_command("iptables");
    let ip6tables = has_command("ip6tables");
    let ip6tables_mangle = has_table("ip6tables", "mangle");
    Capabilities {
        iptables,
        ip6tables,
        ip6tables_nat: has_table("ip6tables", "nat"),
        ip6tables_mangle,
        ipset: has_command("ipset"),
        nft: has_command("nft"),
        tun: Path::new("/dev/net/tun").exists(),
        api_level: api_level(),
        tproxy_v4: iptables && has_kernel_target("/proc/net/ip_tables_targets", "TPROXY"),
        tproxy_v6: ip6tables
            && ip6tables_mangle
            && has_kernel_target("/proc/net/ip6_tables_targets", "TPROXY"),
        socket_transparent: iptables && has_kernel_match("/proc/net/ip_tables_matches", "socket"),
    }
}

fn has_kernel_target(proc_path: &str, name: &str) -> bool {
    match fs::read_to_string(proc_path) {
        Ok(text) => text.lines().any(|line| line.trim() == name),
        // Non-root test environments or hosts without /proc/net/ip_tables_targets: assume supported
        // if iptables command exists so unit tests and minimal containers still work.
        Err(_) => true,
    }
}

fn has_kernel_match(proc_path: &str, name: &str) -> bool {
    match fs::read_to_string(proc_path) {
        Ok(text) => text.lines().any(|line| line.trim() == name),
        Err(_) => false,
    }
}

#[cfg(test)]
pub fn plan(settings: &Value, caps: Capabilities) -> Result<Plan> {
    plan_with_dns(settings, caps, None)
}

#[cfg(test)]
pub fn plan_with_dns(settings: &Value, caps: Capabilities, dns_port: Option<u16>) -> Result<Plan> {
    plan_with_dns_and_rules(settings, caps, dns_port, None)
}

#[derive(Debug, Clone)]
struct PlanContext<'a> {
    mode: &'a str,
    effective: &'a str,
    tproxy_port: u16,
    redir_port: u16,
    dns_port: Option<u16>,
    ipv6: bool,
    proxy_tcp: bool,
    proxy_udp: bool,
    proxy_mode: &'a str,
    proxy_uids: Vec<u32>,
    proxy_gids: Vec<u32>,
    allow_aps: Vec<String>,
    ignore_aps: Vec<String>,
    mac_filter: bool,
    mac_mode: &'a str,
    macs_list: Vec<String>,
    force_v4: Vec<String>,
    force_v6: Vec<String>,
    force_uids: Vec<u32>,
    cnip_v4: bool,
    cnip_v6: bool,
    dns_hijack_mode: &'a str,
    mihomo_dns_forward: bool,
    dns_hijack_tcp: bool,
    dns_hijack_udp: bool,
    performance_mode: bool,
    tun_device: String,
    fake_ip_v4: Option<String>,
    fake_ip_v6: Option<String>,
    caps: Capabilities,
    document: Option<&'a NetworkRuleDocument>,
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
    let ipv6 = enabled(settings, "ipv6");
    let proxy_tcp = enabled(settings, "proxy_tcp");
    let proxy_udp = enabled(settings, "proxy_udp");
    let bypass_cn = enabled(settings, "bypass_cn_ip");
    let bypass_cn_v4_flag = settings
        .get("bypass_cn_ip_v4")
        .and_then(Value::as_str)
        .map(|v| v == "true")
        .unwrap_or(bypass_cn);
    let bypass_cn_v6_flag = settings
        .get("bypass_cn_ip_v6")
        .and_then(Value::as_str)
        .map(|v| v == "true")
        .unwrap_or(false);
    let cnip_v4 = bypass_cn && bypass_cn_v4_flag && caps.ipset;
    let cnip_v6 = bypass_cn && ipv6 && bypass_cn_v6_flag && caps.ipset;
    let force_uids = if cnip_v4 || cnip_v6 {
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
    if matches!(mode, "tproxy" | "enhance") && caps.iptables && !caps.tproxy_v4 {
        plan.effective_mode = "redirect".to_string();
        plan.degraded = true;
        plan.warnings.push(
            "kernel TPROXY target is unavailable; degraded network mode to redirect".to_string(),
        );
    }

    let raw_dns_mode = settings
        .get("dns_hijack_mode")
        .and_then(Value::as_str)
        .unwrap_or("tproxy");
    let effective_dns_mode = if raw_dns_mode == "tproxy" && !caps.tproxy_v4 {
        "redirect"
    } else {
        raw_dns_mode
    };
    let mihomo_dns_forward = settings
        .get("mihomo_dns_forward")
        .and_then(Value::as_str)
        .map(|v| v == "enable")
        .unwrap_or(true);

    let effective = plan.effective_mode.clone();
    let ctx = PlanContext {
        mode,
        effective: effective.as_str(),
        tproxy_port,
        redir_port,
        dns_port,
        ipv6,
        proxy_tcp,
        proxy_udp,
        proxy_mode: settings
            .get("proxy_mode")
            .and_then(Value::as_str)
            .unwrap_or("core"),
        proxy_uids: u32_list(settings, "__proxy_uids"),
        proxy_gids: u32_list(settings, "__proxy_gids"),
        allow_aps: string_list(settings, "__ap_allow"),
        ignore_aps: string_list(settings, "__ap_ignore"),
        mac_filter: enabled(settings, "mac_filter"),
        mac_mode: settings
            .get("mac_mode")
            .and_then(Value::as_str)
            .unwrap_or("blacklist"),
        macs_list: string_list(settings, "macs_list"),
        force_v4: string_list(settings, "force_proxy_cidrs"),
        force_v6: string_list(settings, "force_proxy_cidrs6"),
        force_uids,
        cnip_v4,
        cnip_v6,
        dns_hijack_mode: effective_dns_mode,
        mihomo_dns_forward,
        dns_hijack_tcp: enabled(settings, "dns_hijack_tcp"),
        dns_hijack_udp: enabled(settings, "dns_hijack_udp"),
        performance_mode: enabled(settings, "performance_mode"),
        tun_device: settings
            .get("__tun_device")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .unwrap_or("meta")
            .to_string(),
        fake_ip_v4: settings
            .get("__fake_ip_range")
            .and_then(Value::as_str)
            .map(str::to_string),
        fake_ip_v6: settings
            .get("__fake_ip_range6")
            .and_then(Value::as_str)
            .map(str::to_string),
        caps,
        document: rules,
    };

    if !caps.iptables && caps.nft {
        return nft_plan(&mut plan, &ctx);
    }
    if !caps.iptables {
        plan.warnings
            .push("iptables is unavailable; no rules can be applied".to_string());
        plan.degraded = true;
        return Ok(plan);
    }

    if ctx.cnip_v4 || ctx.cnip_v6 {
        plan.rules.extend(ipset_create_rules(ctx.cnip_v4, ctx.cnip_v6));
    }

    if effective == "tun" {
        if !caps.tun {
            plan.warnings
                .push("TUN device is unavailable; cannot apply TUN mode".to_string());
            plan.degraded = true;
        }
        plan.rules
            .extend(tun_forward_accept_rules("iptables", &ctx.tun_device));
        if ipv6 && caps.ip6tables {
            plan.rules
                .extend(tun_forward_accept_rules("ip6tables", &ctx.tun_device));
        }
        // Only install custom mangle TUN bypass + table 2025 when bypass_cn_ip=true.
        // Otherwise mihomo auto-route owns TUN routing (legacy box.iptables:3053-3066).
        if bypass_cn {
            plan.rules.extend(local_ip_chain_init("iptables", "mangle", "LOCAL_IP_V4"));
            plan.rules.extend(tun_bypass_rules("iptables", &ctx, ctx.cnip_v4.then_some("cnip"), &ctx.force_v4));
            if ipv6 && caps.ip6tables && caps.ip6tables_mangle {
                plan.rules.extend(local_ip_chain_init("ip6tables", "mangle", "LOCAL_IP_V6"));
                plan.rules.extend(tun_bypass_rules(
                    "ip6tables",
                    &ctx,
                    ctx.cnip_v6.then_some("cnip6"),
                    &ctx.force_v6,
                ));
            }
        }
        plan.rules.extend(quic_block_rules(settings, &ctx));
        plan.rules.extend(user_filter_rules(rules, mode, "iptables"));
        if ipv6 && caps.ip6tables {
            plan.rules
                .extend(user_filter_rules(rules, mode, "ip6tables"));
        }
        return Ok(plan);
    }

    // Create dynamic LOCAL_IP_V4 / LOCAL_IP_V6 chains in tables we touch.
    let uses_mangle = matches!(effective.as_str(), "tproxy" | "enhance");
    let uses_nat = matches!(effective.as_str(), "redirect" | "mixed" | "enhance");
    if uses_mangle {
        plan.rules
            .extend(local_ip_chain_init("iptables", "mangle", "LOCAL_IP_V4"));
        if ipv6 && caps.ip6tables && caps.ip6tables_mangle && caps.tproxy_v6 {
            plan.rules
                .extend(local_ip_chain_init("ip6tables", "mangle", "LOCAL_IP_V6"));
        }
    }
    if uses_nat {
        plan.rules
            .extend(local_ip_chain_init("iptables", "nat", "LOCAL_IP_V4"));
        if ipv6 && caps.ip6tables && caps.ip6tables_nat {
            plan.rules
                .extend(local_ip_chain_init("ip6tables", "nat", "LOCAL_IP_V6"));
        }
    }

    // Install NAT DNS hijack BEFORE redirect chains in nat/OUTPUT and nat/PREROUTING
    // so TCP port 53 hits BOX_DNS rather than being intercepted by BOX_LOCAL_REDIRECT.
    let need_nat_dns = should_emit_nat_dns(&ctx);
    if let (Some(port), Some((nat_udp, nat_tcp))) = (ctx.dns_port, need_nat_dns) {
        plan.rules
            .extend(dns_rules("iptables", port, nat_udp, nat_tcp, &ctx));
        if ipv6 && caps.ip6tables && caps.ip6tables_nat {
            plan.rules
                .extend(dns_rules("ip6tables", port, nat_udp, nat_tcp, &ctx));
        }
    }

    // Fake-IP ICMP DNAT (IPv4 nat)
    if let Some(ref fake_ip) = ctx.fake_ip_v4 {
        plan.rules.extend(fake_ip_icmp_rules(fake_ip));
    }

    // mangle TPROXY (tproxy = TCP+UDP; enhance = UDP only!)
    if uses_mangle {
        let mangle_tcp = effective != "enhance" && ctx.proxy_tcp;
        let mangle_udp = ctx.proxy_udp;
        if mangle_tcp || mangle_udp {
            plan.rules.extend(tproxy_family_rules(
                "iptables",
                &ctx,
                mangle_tcp,
                mangle_udp,
                ctx.cnip_v4.then_some("cnip"),
                &ctx.force_v4,
                ctx.fake_ip_v4.as_deref(),
            ));
            if ipv6 && caps.ip6tables && caps.ip6tables_mangle && caps.tproxy_v6 {
                plan.rules.extend(tproxy_family_rules(
                    "ip6tables",
                    &ctx,
                    mangle_tcp,
                    mangle_udp,
                    ctx.cnip_v6.then_some("cnip6"),
                    &ctx.force_v6,
                    ctx.fake_ip_v6.as_deref(),
                ));
            } else if ipv6 && caps.ip6tables {
                plan.warnings.push(
                    "ip6tables mangle/TPROXY is unavailable; IPv6 tproxy rules are skipped"
                        .to_string(),
                );
                plan.degraded = true;
            }
        }
        plan.rules
            .extend(loopback_reject_rules("iptables", ctx.tproxy_port));
        if ipv6 && caps.ip6tables {
            plan.rules
                .extend(loopback_reject_rules("ip6tables", ctx.tproxy_port));
        }
    }

    // nat REDIRECT (redirect, mixed, enhance: TCP only)
    if uses_nat {
        if ctx.proxy_tcp {
            plan.rules.extend(redirect_family_rules(
                "iptables",
                &ctx,
                ctx.cnip_v4.then_some("cnip"),
                &ctx.force_v4,
            ));
            if ipv6 && caps.ip6tables && caps.ip6tables_nat {
                plan.rules.extend(redirect_family_rules(
                    "ip6tables",
                    &ctx,
                    ctx.cnip_v6.then_some("cnip6"),
                    &ctx.force_v6,
                ));
            } else if ipv6 && caps.ip6tables && !caps.ip6tables_nat {
                plan.warnings.push(
                    "ip6tables nat table is unavailable; IPv6 redirect rules are skipped"
                        .to_string(),
                );
                plan.degraded = true;
            }
        }
        plan.rules
            .extend(loopback_reject_rules("iptables", ctx.redir_port));
        if ipv6 && caps.ip6tables {
            plan.rules
                .extend(loopback_reject_rules("ip6tables", ctx.redir_port));
        }
    }

    if effective == "redirect" && ctx.proxy_udp {
        plan.warnings.push(
            "local UDP transparent proxy is unavailable in redirect; local UDP remains direct"
                .to_string(),
        );
        plan.degraded = true;
    }

    // mixed = TUN (UDP via mihomo TUN) + redirect (TCP via nat)
    if effective == "mixed" {
        if !caps.tun {
            plan.warnings
                .push("TUN device is unavailable; mixed mode UDP path degraded".to_string());
            plan.degraded = true;
        }
        plan.rules
            .extend(tun_forward_accept_rules("iptables", &ctx.tun_device));
        if ipv6 && caps.ip6tables {
            plan.rules
                .extend(tun_forward_accept_rules("ip6tables", &ctx.tun_device));
        }
        if bypass_cn {
            plan.rules
                .extend(local_ip_chain_init("iptables", "mangle", "LOCAL_IP_V4"));
            plan.rules.extend(tun_bypass_rules(
                "iptables",
                &ctx,
                ctx.cnip_v4.then_some("cnip"),
                &ctx.force_v4,
            ));
            if ipv6 && caps.ip6tables && caps.ip6tables_mangle {
                plan.rules
                    .extend(local_ip_chain_init("ip6tables", "mangle", "LOCAL_IP_V6"));
                plan.rules.extend(tun_bypass_rules(
                    "ip6tables",
                    &ctx,
                    ctx.cnip_v6.then_some("cnip6"),
                    &ctx.force_v6,
                ));
            }
        }
    }

    plan.rules.extend(quic_block_rules(settings, &ctx));
    plan.rules.extend(user_filter_rules(rules, mode, "iptables"));
    if ipv6 && caps.ip6tables {
        plan.rules
            .extend(user_filter_rules(rules, mode, "ip6tables"));
    }
    Ok(plan)
}

fn should_emit_nat_dns(ctx: &PlanContext<'_>) -> Option<(bool, bool)> {
    ctx.dns_port?;
    if ctx.dns_hijack_mode == "disable" {
        return None;
    }
    if ctx.dns_hijack_mode == "redirect" {
        if ctx.dns_hijack_udp || ctx.dns_hijack_tcp {
            return Some((ctx.dns_hijack_udp, ctx.dns_hijack_tcp));
        }
        return None;
    }
    // dns_hijack_mode == "tproxy":
    // - In redirect/mixed (no mangle UDP TPROXY), fall back to NAT DNS redirect so DNS does not leak.
    // - In tproxy/enhance with mihomo_dns_forward=enable, redirect UDP 53 via NAT to mihomo DNS port
    //   (and return UDP 53 in mangle so it is not double-handled).
    if matches!(ctx.effective, "redirect" | "mixed") {
        if ctx.dns_hijack_udp || ctx.dns_hijack_tcp {
            return Some((ctx.dns_hijack_udp, ctx.dns_hijack_tcp));
        }
        return None;
    }
    if ctx.mihomo_dns_forward && ctx.dns_hijack_udp {
        return Some((true, false));
    }
    None
}

fn local_ip_chain_init(family: &'static str, table: &'static str, chain: &'static str) -> Vec<Rule> {
    vec![Rule {
        family,
        table,
        chain,
        args: vec!["-N".to_string(), chain.to_string()],
    }]
}

fn tun_forward_accept_rules(family: &'static str, tun_device: &str) -> Vec<Rule> {
    vec![
        Rule {
            family,
            table: "filter",
            chain: "FORWARD",
            args: vec![
                "-I".to_string(),
                "FORWARD".to_string(),
                "-i".to_string(),
                tun_device.to_string(),
                "-j".to_string(),
                "ACCEPT".to_string(),
            ],
        },
        Rule {
            family,
            table: "filter",
            chain: "FORWARD",
            args: vec![
                "-I".to_string(),
                "FORWARD".to_string(),
                "-o".to_string(),
                tun_device.to_string(),
                "-j".to_string(),
                "ACCEPT".to_string(),
            ],
        },
    ]
}

fn loopback_reject_rules(family: &'static str, port: u16) -> Vec<Rule> {
    let loopback = if family == "ip6tables" {
        "::1/128"
    } else {
        "127.0.0.1/32"
    };
    vec![Rule {
        family,
        table: "filter",
        chain: "OUTPUT",
        args: vec![
            "-A".to_string(),
            "OUTPUT".to_string(),
            "-d".to_string(),
            loopback.to_string(),
            "-p".to_string(),
            "tcp".to_string(),
            "-m".to_string(),
            "owner".to_string(),
            "--uid-owner".to_string(),
            CORE_UID.to_string(),
            "--gid-owner".to_string(),
            CORE_GID.to_string(),
            "--dport".to_string(),
            port.to_string(),
            "-j".to_string(),
            "REJECT".to_string(),
        ],
    }]
}

fn fake_ip_icmp_rules(fake_ip_range: &str) -> Vec<Rule> {
    vec![
        Rule {
            family: "iptables",
            table: "nat",
            chain: "OUTPUT",
            args: vec![
                "-I".to_string(),
                "OUTPUT".to_string(),
                "-d".to_string(),
                fake_ip_range.to_string(),
                "-p".to_string(),
                "icmp".to_string(),
                "-j".to_string(),
                "DNAT".to_string(),
                "--to-destination".to_string(),
                "127.0.0.1".to_string(),
            ],
        },
        Rule {
            family: "iptables",
            table: "nat",
            chain: "PREROUTING",
            args: vec![
                "-I".to_string(),
                "PREROUTING".to_string(),
                "-d".to_string(),
                fake_ip_range.to_string(),
                "-p".to_string(),
                "icmp".to_string(),
                "-j".to_string(),
                "DNAT".to_string(),
                "--to-destination".to_string(),
                "127.0.0.1".to_string(),
            ],
        },
    ]
}

fn core_owner_bypass_rule(
    family: &'static str,
    table: &'static str,
    chain: &'static str,
) -> Rule {
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
            CORE_UID.to_string(),
            "--gid-owner".to_string(),
            CORE_GID.to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    }
}

fn local_ip_jump(family: &'static str, table: &'static str, chain: &'static str) -> Rule {
    let target = if family == "ip6tables" {
        "LOCAL_IP_V6"
    } else {
        "LOCAL_IP_V4"
    };
    Rule {
        family,
        table,
        chain,
        args: vec![
            "-A".to_string(),
            chain.to_string(),
            "-j".to_string(),
            target.to_string(),
        ],
    }
}

fn performance_divert_rules(family: &'static str) -> Vec<Rule> {
    vec![
        Rule {
            family,
            table: "mangle",
            chain: "DIVERT",
            args: vec!["-N".to_string(), "DIVERT".to_string()],
        },
        Rule {
            family,
            table: "mangle",
            chain: "DIVERT",
            args: vec![
                "-A".to_string(),
                "DIVERT".to_string(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                TPROXY_MARK.to_string(),
            ],
        },
        Rule {
            family,
            table: "mangle",
            chain: "DIVERT",
            args: vec![
                "-A".to_string(),
                "DIVERT".to_string(),
                "-j".to_string(),
                "ACCEPT".to_string(),
            ],
        },
        Rule {
            family,
            table: "mangle",
            chain: "PREROUTING",
            args: vec![
                "-I".to_string(),
                "PREROUTING".to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "-m".to_string(),
                "socket".to_string(),
                "--transparent".to_string(),
                "-j".to_string(),
                "DIVERT".to_string(),
            ],
        },
    ]
}

fn mangle_dns_bypass_rules(
    family: &'static str,
    chain: &'static str,
    ctx: &PlanContext<'_>,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    let skip_udp = ctx.dns_hijack_mode == "disable"
        || ctx.dns_hijack_mode == "redirect"
        || (ctx.dns_hijack_mode == "tproxy" && ctx.mihomo_dns_forward)
        || !ctx.dns_hijack_udp;
    let skip_tcp = ctx.dns_hijack_mode == "disable"
        || ctx.dns_hijack_mode == "redirect"
        || !ctx.dns_hijack_tcp;
    if skip_udp {
        rules.push(Rule {
            family,
            table: "mangle",
            chain,
            args: vec![
                "-A".to_string(),
                chain.to_string(),
                "-p".to_string(),
                "udp".to_string(),
                "--dport".to_string(),
                "53".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    if skip_tcp {
        rules.push(Rule {
            family,
            table: "mangle",
            chain,
            args: vec![
                "-A".to_string(),
                chain.to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "--dport".to_string(),
                "53".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    rules
}

fn tproxy_family_rules(
    family: &'static str,
    ctx: &PlanContext<'_>,
    proxy_tcp: bool,
    proxy_udp: bool,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
    fake_ip: Option<&str>,
) -> Vec<Rule> {
    let mut rules = Vec::new();

    if ctx.performance_mode && ctx.caps.socket_transparent {
        rules.extend(performance_divert_rules(family));
    }

    // --- PREROUTING chain: BOX_TPROXY ---
    let pre = "BOX_TPROXY";
    rules.push(Rule {
        family,
        table: "mangle",
        chain: pre,
        args: vec!["-N".to_string(), pre.to_string()],
    });
    if ctx.performance_mode {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: pre,
            args: vec![
                "-A".to_string(),
                pre.to_string(),
                "-m".to_string(),
                "conntrack".to_string(),
                "--ctstate".to_string(),
                "ESTABLISHED,RELATED".to_string(),
                "-m".to_string(),
                "conntrack".to_string(),
                "--ctdir".to_string(),
                "REPLY".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    // Ignored inbound interfaces
    for iface in &ctx.ignore_aps {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: pre,
            args: vec![
                "-A".to_string(),
                pre.to_string(),
                "-i".to_string(),
                iface.clone(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    // DNS return in mangle when NAT handles DNS or DNS hijack is disabled
    rules.extend(mangle_dns_bypass_rules(family, pre, ctx));

    // IPv6 Fake-IP range must be proxied BEFORE private/ULA bypasses (fc00::/7)
    if family == "ip6tables" {
        if let Some(fake6) = fake_ip {
            for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
                if !enabled_flag {
                    continue;
                }
                rules.push(tproxy_dispatch_rule(
                    family,
                    pre,
                    vec!["-d".to_string(), fake6.to_string(), "-p".to_string(), proto.to_string()],
                    ctx.tproxy_port,
                ));
            }
        }
    }

    // Force-proxy destination CIDRs BEFORE local/CNIP bypass
    for cidr in force_cidrs {
        for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled_flag {
                continue;
            }
            rules.push(tproxy_dispatch_rule(
                family,
                pre,
                vec!["-d".to_string(), cidr.clone(), "-p".to_string(), proto.to_string()],
                ctx.tproxy_port,
            ));
        }
    }

    // Dynamic local IP bypass & user/default forward bypass rules
    rules.push(local_ip_jump(family, "mangle", pre));
    rules.extend(user_rules_for(
        ctx.document,
        ctx.mode,
        family,
        Scope::Forward,
        "mangle",
        pre,
        false,
    ));
    if let Some(set) = cnip_set {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: pre,
            args: vec![
                "-A".to_string(),
                pre.to_string(),
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
    // NOTE: Do NOT emit `-m mark --mark 0x01000000 -j RETURN` here!
    // Packets rerouted from OUTPUT -> lo carry 0x01000000 and MUST reach TPROXY below.
    // Always protect ADB wireless debugging port if not already covered by default rules.
    if !has_adb_default_rule(ctx.document) {
        rules.extend(adb_port_bypass_rules(family, "mangle", pre, true, false));
    }

    // Dispatch loopback (rerouted local OUTPUT packets) and allowed AP/hotspot interfaces.
    // If no ap.list.cfg allow entries exist, dispatch lo + any inbound traffic cleanly.
    if ctx.allow_aps.is_empty() {
        for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled_flag {
                continue;
            }
            rules.push(tproxy_dispatch_rule(
                family,
                pre,
                vec!["-p".to_string(), proto.to_string()],
                ctx.tproxy_port,
            ));
        }
    } else {
        for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled_flag {
                continue;
            }
            rules.push(tproxy_dispatch_rule(
                family,
                pre,
                vec!["-p".to_string(), proto.to_string(), "-i".to_string(), "lo".to_string()],
                ctx.tproxy_port,
            ));
        }
        for ap in &ctx.allow_aps {
            rules.extend(ap_dispatch_rules(
                family,
                "mangle",
                pre,
                ap,
                true,
                proxy_tcp,
                proxy_udp,
                ctx.tproxy_port,
                ctx,
            ));
        }
    }

    rules.push(Rule {
        family,
        table: "mangle",
        chain: "PREROUTING",
        args: vec![
            "-I".to_string(),
            "PREROUTING".to_string(),
            "-j".to_string(),
            pre.to_string(),
        ],
    });

    // --- OUTPUT chain: BOX_TPROXY_LOCAL (pure mangle local TCP+UDP or enhance UDP-only) ---
    let local = "BOX_TPROXY_LOCAL";
    rules.push(Rule {
        family,
        table: "mangle",
        chain: local,
        args: vec!["-N".to_string(), local.to_string()],
    });
    rules.push(core_owner_bypass_rule(family, "mangle", local));
    if ctx.performance_mode {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: local,
            args: vec![
                "-A".to_string(),
                local.to_string(),
                "-m".to_string(),
                "conntrack".to_string(),
                "--ctstate".to_string(),
                "ESTABLISHED,RELATED".to_string(),
                "-m".to_string(),
                "conntrack".to_string(),
                "--ctdir".to_string(),
                "REPLY".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    rules.extend(mangle_dns_bypass_rules(family, local, ctx));
    if !has_adb_default_rule(ctx.document) {
        rules.extend(adb_port_bypass_rules(family, "mangle", local, proxy_tcp, proxy_udp));
    }
    // IPv6 Fake-IP range must be marked BEFORE private/ULA bypasses
    if family == "ip6tables" {
        if let Some(fake6) = fake_ip {
            for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
                if !enabled_flag {
                    continue;
                }
                rules.push(mangle_mark_rule(
                    family,
                    local,
                    vec!["-d".to_string(), fake6.to_string(), "-p".to_string(), proto.to_string()],
                ));
            }
        }
    }
    // Force-proxy destination CIDRs BEFORE local/CNIP bypass
    for cidr in force_cidrs {
        for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled_flag {
                continue;
            }
            rules.push(mangle_mark_rule(
                family,
                local,
                vec!["-d".to_string(), cidr.clone(), "-p".to_string(), proto.to_string()],
            ));
        }
    }
    // CNIP force-proxy apps BEFORE CNIP bypass (never replaces normal local rules!)
    if let Some(set) = cnip_set {
        for uid in &ctx.force_uids {
            for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
                if !enabled_flag {
                    continue;
                }
                rules.push(mangle_mark_rule(
                    family,
                    local,
                    vec![
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
                    ],
                ));
                rules.push(Rule {
                    family,
                    table: "mangle",
                    chain: local,
                    args: vec![
                        "-A".to_string(),
                        local.to_string(),
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
    }
    rules.push(local_ip_jump(family, "mangle", local));
    rules.extend(user_rules_for(
        ctx.document,
        ctx.mode,
        family,
        Scope::Local,
        "mangle",
        local,
        true,
    ));
    if let Some(set) = cnip_set {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: local,
            args: vec![
                "-A".to_string(),
                local.to_string(),
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
    emit_proxy_mode_rules(
        &mut rules,
        family,
        "mangle",
        local,
        "mark",
        ctx.tproxy_port,
        proxy_tcp,
        proxy_udp,
        ctx.proxy_mode,
        &ctx.proxy_uids,
        &ctx.proxy_gids,
    );
    rules.push(Rule {
        family,
        table: "mangle",
        chain: "OUTPUT",
        args: vec![
            "-I".to_string(),
            "OUTPUT".to_string(),
            "-j".to_string(),
            local.to_string(),
        ],
    });

    // Policy routing: pref 100 table 2024
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
            TPROXY_MARK.to_string(),
            "table".to_string(),
            TPROXY_TABLE.to_string(),
            "pref".to_string(),
            TPROXY_PREF.to_string(),
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
            TPROXY_TABLE.to_string(),
        ],
    });

    rules
}

fn redirect_family_rules(
    family: &'static str,
    ctx: &PlanContext<'_>,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
) -> Vec<Rule> {
    let mut rules = Vec::new();

    // --- PREROUTING chain: BOX_REDIRECT (no xt_socket in nat!) ---
    let pre = "BOX_REDIRECT";
    rules.push(Rule {
        family,
        table: "nat",
        chain: pre,
        args: vec!["-N".to_string(), pre.to_string()],
    });
    for iface in &ctx.ignore_aps {
        rules.push(Rule {
            family,
            table: "nat",
            chain: pre,
            args: vec![
                "-A".to_string(),
                pre.to_string(),
                "-i".to_string(),
                iface.clone(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    // Skip port 53 in BOX_REDIRECT when NAT DNS is active (handled by BOX_DNS_PRE) or disabled
    if ctx.dns_hijack_mode == "disable" || ctx.dns_port.is_some() {
        rules.push(Rule {
            family,
            table: "nat",
            chain: pre,
            args: vec![
                "-A".to_string(),
                pre.to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "--dport".to_string(),
                "53".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    for cidr in force_cidrs {
        rules.push(redirect_dispatch_rule(
            family,
            pre,
            vec!["-d".to_string(), cidr.clone(), "-p".to_string(), "tcp".to_string()],
            ctx.redir_port,
        ));
    }
    rules.push(local_ip_jump(family, "nat", pre));
    rules.extend(user_rules_for(
        ctx.document,
        ctx.mode,
        family,
        Scope::Forward,
        "nat",
        pre,
        false,
    ));
    if let Some(set) = cnip_set {
        rules.push(Rule {
            family,
            table: "nat",
            chain: pre,
            args: vec![
                "-A".to_string(),
                pre.to_string(),
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
    if !has_adb_default_rule(ctx.document) {
        rules.extend(adb_port_bypass_rules(family, "nat", pre, true, false));
    }
    if ctx.allow_aps.is_empty() {
        rules.push(redirect_dispatch_rule(
            family,
            pre,
            vec!["-p".to_string(), "tcp".to_string()],
            ctx.redir_port,
        ));
    } else {
        rules.push(redirect_dispatch_rule(
            family,
            pre,
            vec!["-p".to_string(), "tcp".to_string(), "-i".to_string(), "lo".to_string()],
            ctx.redir_port,
        ));
        for ap in &ctx.allow_aps {
            rules.extend(ap_dispatch_rules(
                family,
                "nat",
                pre,
                ap,
                false,
                true,
                false,
                ctx.redir_port,
                ctx,
            ));
        }
    }
    // Append AFTER BOX_DNS_PRE (which was appended earlier in PREROUTING)
    rules.push(Rule {
        family,
        table: "nat",
        chain: "PREROUTING",
        args: vec![
            "-A".to_string(),
            "PREROUTING".to_string(),
            "-j".to_string(),
            pre.to_string(),
        ],
    });

    // --- OUTPUT chain: BOX_LOCAL_REDIRECT ---
    let local = "BOX_LOCAL_REDIRECT";
    rules.push(Rule {
        family,
        table: "nat",
        chain: local,
        args: vec!["-N".to_string(), local.to_string()],
    });
    rules.push(core_owner_bypass_rule(family, "nat", local));
    if ctx.dns_hijack_mode == "disable" || ctx.dns_port.is_some() {
        rules.push(Rule {
            family,
            table: "nat",
            chain: local,
            args: vec![
                "-A".to_string(),
                local.to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "--dport".to_string(),
                "53".to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
    if !has_adb_default_rule(ctx.document) {
        rules.extend(adb_port_bypass_rules(family, "nat", local, true, false));
    }
    for cidr in force_cidrs {
        rules.push(redirect_dispatch_rule(
            family,
            local,
            vec!["-d".to_string(), cidr.clone(), "-p".to_string(), "tcp".to_string()],
            ctx.redir_port,
        ));
    }
    if let Some(set) = cnip_set {
        for uid in &ctx.force_uids {
            rules.push(redirect_dispatch_rule(
                family,
                local,
                vec![
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
                ],
                ctx.redir_port,
            ));
        }
    }
    rules.push(local_ip_jump(family, "nat", local));
    rules.extend(user_rules_for(
        ctx.document,
        ctx.mode,
        family,
        Scope::Local,
        "nat",
        local,
        true,
    ));
    if let Some(set) = cnip_set {
        rules.push(Rule {
            family,
            table: "nat",
            chain: local,
            args: vec![
                "-A".to_string(),
                local.to_string(),
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
    emit_proxy_mode_rules(
        &mut rules,
        family,
        "nat",
        local,
        "redirect",
        ctx.redir_port,
        true,
        false,
        ctx.proxy_mode,
        &ctx.proxy_uids,
        &ctx.proxy_gids,
    );
    // Append AFTER BOX_DNS (which was appended earlier in OUTPUT)
    rules.push(Rule {
        family,
        table: "nat",
        chain: "OUTPUT",
        args: vec![
            "-A".to_string(),
            "OUTPUT".to_string(),
            "-j".to_string(),
            local.to_string(),
        ],
    });

    rules
}

#[allow(clippy::too_many_arguments)]
fn ap_dispatch_rules(
    family: &'static str,
    table: &'static str,
    chain: &'static str,
    ap: &str,
    is_tproxy: bool,
    proxy_tcp: bool,
    proxy_udp: bool,
    port: u16,
    ctx: &PlanContext<'_>,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    let valid_macs: Vec<String> = ctx
        .macs_list
        .iter()
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| is_mac_address(m))
        .collect();
    let emit = |rules: &mut Vec<Rule>, mac: Option<&str>| {
        for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled_flag {
                continue;
            }
            let mut prefix = vec![
                "-p".to_string(),
                proto.to_string(),
                "-i".to_string(),
                ap.to_string(),
            ];
            if let Some(mac) = mac {
                prefix.extend([
                    "-m".to_string(),
                    "mac".to_string(),
                    "--mac-source".to_string(),
                    mac.to_string(),
                ]);
            }
            if is_tproxy {
                rules.push(tproxy_dispatch_rule(family, chain, prefix, port));
            } else {
                rules.push(redirect_dispatch_rule(family, chain, prefix, port));
            }
        }
    };

    if ctx.mac_filter {
        if ctx.mac_mode == "whitelist" {
            for mac in &valid_macs {
                emit(&mut rules, Some(mac));
            }
        } else {
            for mac in &valid_macs {
                if !is_tproxy {
                    rules.push(Rule {
                        family,
                        table,
                        chain,
                        args: vec![
                            "-A".to_string(),
                            chain.to_string(),
                            "-i".to_string(),
                            ap.to_string(),
                            "-m".to_string(),
                            "mac".to_string(),
                            "--mac-source".to_string(),
                            mac.clone(),
                            "-j".to_string(),
                            "RETURN".to_string(),
                        ],
                    });
                } else {
                    for (proto, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
                        if !enabled_flag {
                            continue;
                        }
                        rules.push(Rule {
                            family,
                            table,
                            chain,
                            args: vec![
                                "-A".to_string(),
                                chain.to_string(),
                                "-p".to_string(),
                                proto.to_string(),
                                "-i".to_string(),
                                ap.to_string(),
                                "-m".to_string(),
                                "mac".to_string(),
                                "--mac-source".to_string(),
                                mac.clone(),
                                "-j".to_string(),
                                "RETURN".to_string(),
                            ],
                        });
                    }
                }
            }
            emit(&mut rules, None);
        }
    } else {
        emit(&mut rules, None);
    }
    rules
}

fn is_mac_address(value: &str) -> bool {
    let parts: Vec<&str> = value.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn tproxy_dispatch_rule(
    family: &'static str,
    chain: &'static str,
    mut match_args: Vec<String>,
    port: u16,
) -> Rule {
    let mut args = vec!["-A".to_string(), chain.to_string()];
    args.append(&mut match_args);
    args.extend([
        "-j".to_string(),
        "TPROXY".to_string(),
        "--on-port".to_string(),
        port.to_string(),
        "--tproxy-mark".to_string(),
        TPROXY_MARK.to_string(),
    ]);
    Rule {
        family,
        table: "mangle",
        chain,
        args,
    }
}

fn mangle_mark_rule(
    family: &'static str,
    chain: &'static str,
    mut match_args: Vec<String>,
) -> Rule {
    let mut args = vec!["-A".to_string(), chain.to_string()];
    args.append(&mut match_args);
    args.extend([
        "-j".to_string(),
        "MARK".to_string(),
        "--set-xmark".to_string(),
        TPROXY_MARK.to_string(),
    ]);
    Rule {
        family,
        table: "mangle",
        chain,
        args,
    }
}

fn redirect_dispatch_rule(
    family: &'static str,
    chain: &'static str,
    mut match_args: Vec<String>,
    port: u16,
) -> Rule {
    let mut args = vec!["-A".to_string(), chain.to_string()];
    args.append(&mut match_args);
    args.extend([
        "-j".to_string(),
        "REDIRECT".to_string(),
        "--to-ports".to_string(),
        port.to_string(),
    ]);
    Rule {
        family,
        table: "nat",
        chain,
        args,
    }
}

fn has_adb_default_rule(document: Option<&NetworkRuleDocument>) -> bool {
    document.is_some_and(|doc| {
        doc.rules
            .iter()
            .any(|r| r.enabled && r.match_fields.dst_ports.iter().any(|p| p == "5555"))
    })
}

fn adb_port_bypass_rules(
    family: &'static str,
    table: &'static str,
    chain: &'static str,
    tcp: bool,
    udp: bool,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    for (proto, enabled_flag) in [("tcp", tcp), ("udp", udp)] {
        if !enabled_flag {
            continue;
        }
        for direction in ["--dport", "--sport"] {
            rules.push(Rule {
                family,
                table,
                chain,
                args: vec![
                    "-A".to_string(),
                    chain.to_string(),
                    "-p".to_string(),
                    proto.to_string(),
                    direction.to_string(),
                    "5555".to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            });
        }
    }
    rules
}

#[allow(clippy::too_many_arguments)]
fn emit_proxy_mode_rules(
    rules: &mut Vec<Rule>,
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
) {
    let target = if action == "mark" { "MARK" } else { "REDIRECT" };
    let push_owner = |rules: &mut Vec<Rule>,
                      protocol: &str,
                      uid: Option<u32>,
                      gid: Option<u32>,
                      tgt: &str| {
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
            tgt.to_string(),
        ]);
        if tgt == "MARK" {
            args.extend(["--set-xmark".to_string(), TPROXY_MARK.to_string()]);
        } else if tgt == "REDIRECT" {
            args.extend(["--to-ports".to_string(), port.to_string()]);
        }
        rules.push(Rule {
            family,
            table,
            chain,
            args,
        });
    };
    let push_all = |rules: &mut Vec<Rule>, protocol: &str, tgt: &str| {
        let mut args = vec![
            "-A".to_string(),
            chain.to_string(),
            "-p".to_string(),
            protocol.to_string(),
            "-j".to_string(),
            tgt.to_string(),
        ];
        if tgt == "MARK" {
            args.extend(["--set-xmark".to_string(), TPROXY_MARK.to_string()]);
        } else if tgt == "REDIRECT" {
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
        for (protocol, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
            if !enabled_flag {
                continue;
            }
            for uid in uids {
                push_owner(rules, protocol, Some(*uid), None, "RETURN");
            }
            for gid in gids {
                push_owner(rules, protocol, None, Some(*gid), "RETURN");
            }
        }
        if proxy_tcp {
            push_all(rules, "tcp", target);
        }
        if proxy_udp {
            push_all(rules, "udp", target);
        }
    } else if proxy_mode == "whitelist" || proxy_mode == "white" {
        if uids.is_empty() && gids.is_empty() {
            if proxy_tcp {
                push_all(rules, "tcp", target);
            }
            if proxy_udp {
                push_all(rules, "udp", target);
            }
        } else {
            // Legacy whitelist explicitly proxies uid 0 (root non-core) & uid 1052 (Android dns/netd).
            let mut effective_uids = BTreeSet::new();
            effective_uids.insert(0u32);
            effective_uids.insert(ANDROID_DNS_UID);
            for uid in uids {
                effective_uids.insert(*uid);
            }
            for (protocol, enabled_flag) in [("tcp", proxy_tcp), ("udp", proxy_udp)] {
                if !enabled_flag {
                    continue;
                }
                for uid in &effective_uids {
                    push_owner(rules, protocol, Some(*uid), None, target);
                }
                for gid in gids {
                    push_owner(rules, protocol, None, Some(*gid), target);
                }
            }
        }
    } else {
        if proxy_tcp {
            push_all(rules, "tcp", target);
        }
        if proxy_udp {
            push_all(rules, "udp", target);
        }
    }
}

fn dns_rules(
    family: &'static str,
    port: u16,
    udp: bool,
    tcp: bool,
    ctx: &PlanContext<'_>,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    for (chain, is_output) in [("BOX_DNS", true), ("BOX_DNS_PRE", false)] {
        rules.push(Rule {
            family,
            table: "nat",
            chain,
            args: vec!["-N".to_string(), chain.to_string()],
        });
        if is_output {
            rules.push(core_owner_bypass_rule(family, "nat", chain));
        } else {
            for iface in &ctx.ignore_aps {
                rules.push(Rule {
                    family,
                    table: "nat",
                    chain,
                    args: vec![
                        "-A".to_string(),
                        chain.to_string(),
                        "-i".to_string(),
                        iface.clone(),
                        "-j".to_string(),
                        "RETURN".to_string(),
                    ],
                });
            }
        }
        rules.extend(user_rules_for(
            ctx.document,
            ctx.mode,
            family,
            Scope::Dns,
            "nat",
            chain,
            is_output,
        ));
        for (protocol, enabled_flag) in [("udp", udp), ("tcp", tcp)] {
            if !enabled_flag {
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

fn ipset_create_rules(v4: bool, v6: bool) -> Vec<Rule> {
    let mut rules = Vec::new();
    if v4 {
        rules.push(Rule {
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
        });
    }
    if v6 {
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

#[allow(clippy::too_many_arguments)]
fn user_rules_for(
    document: Option<&NetworkRuleDocument>,
    mode: &str,
    family: &'static str,
    scope: Scope,
    table: &'static str,
    chain: &'static str,
    is_output: bool,
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
        // Only valid interface directions per hook:
        // - OUTPUT hook allows `-o`, forbids `-i`
        // - PREROUTING hook allows `-i`, forbids `-o`
        // - FORWARD hook allows both `-i` and `-o`
        if is_output {
            if !rule.match_fields.src_ifaces.is_empty() && rule.match_fields.dst_ifaces.is_empty() {
                continue;
            }
            add_values(&mut variants, "-o", &rule.match_fields.dst_ifaces);
        } else if table == "filter" && chain == "BOX_FILTER_FWD" {
            add_values(&mut variants, "-i", &rule.match_fields.src_ifaces);
            add_values(&mut variants, "-o", &rule.match_fields.dst_ifaces);
        } else {
            if !rule.match_fields.dst_ifaces.is_empty() && rule.match_fields.src_ifaces.is_empty() {
                continue;
            }
            add_values(&mut variants, "-i", &rule.match_fields.src_ifaces);
        }
        if !rule.match_fields.uids.is_empty() || !rule.match_fields.gids.is_empty() {
            if !is_output {
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
        if !protocols.is_empty() {
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
        // iptables requires colon `start:end` for port ranges, whereas network-rules stores `start-end`.
        let dst_ports = normalize_iptables_ports(&rule.match_fields.dst_ports);
        let src_ports = normalize_iptables_ports(&rule.match_fields.src_ports);
        add_values(&mut variants, "--dport", &dst_ports);
        add_values(&mut variants, "--sport", &src_ports);
        for mut args in variants {
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

fn normalize_iptables_ports(ports: &[String]) -> Vec<String> {
    ports.iter().map(|port| port.replace('-', ":")).collect()
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
        true,
    );
    // filter table has INPUT / FORWARD / OUTPUT (NO PREROUTING!).
    let forward = user_rules_for(
        Some(document),
        mode,
        family,
        Scope::Forward,
        "filter",
        "BOX_FILTER_FWD",
        false,
    );
    let filter = user_rules_for(
        Some(document),
        mode,
        family,
        Scope::Filter,
        "filter",
        "BOX_FILTER_FWD",
        false,
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
            chain: "BOX_FILTER_FWD",
            args: vec!["-N".to_string(), "BOX_FILTER_FWD".to_string()],
        });
        rules.push(Rule {
            family,
            table: "filter",
            chain: "FORWARD",
            args: vec![
                "-A".to_string(),
                "FORWARD".to_string(),
                "-j".to_string(),
                "BOX_FILTER_FWD".to_string(),
            ],
        });
        rules.extend(forward);
        rules.extend(filter);
    }
    rules
}

fn tun_bypass_rules(
    family: &'static str,
    ctx: &PlanContext<'_>,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
) -> Vec<Rule> {
    let pre = "BOX_TUN_BYPASS_PRE";
    let out = "BOX_TUN_BYPASS_OUT";
    let tun_device = &ctx.tun_device;
    let fake_ip = if family == "ip6tables" {
        ctx.fake_ip_v6.as_deref()
    } else {
        ctx.fake_ip_v4.as_deref()
    };
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
            "-I".to_string(),
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
            "-I".to_string(),
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

    // --- BOX_TUN_BYPASS_PRE ---
    rules.push(mark(
        pre,
        vec![
            "-A".to_string(),
            pre.to_string(),
            "-i".to_string(),
            tun_device.clone(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    ));
    for iface in &ctx.ignore_aps {
        rules.push(mark(
            pre,
            vec![
                "-A".to_string(),
                pre.to_string(),
                "-i".to_string(),
                iface.clone(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    for (proto, enabled_flag) in [("tcp", ctx.dns_hijack_tcp), ("udp", ctx.dns_hijack_udp)] {
        if enabled_flag && ctx.dns_hijack_mode != "disable" {
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
                    TUN_ROUTE_MARK.to_string(),
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
    if let Some(fake) = fake_ip {
        rules.push(mark(
            pre,
            vec![
                "-A".to_string(),
                pre.to_string(),
                "-d".to_string(),
                fake.to_string(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                TUN_ROUTE_MARK.to_string(),
            ],
        ));
        rules.push(mark(
            pre,
            vec![
                "-A".to_string(),
                pre.to_string(),
                "-d".to_string(),
                fake.to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    for cidr in force_cidrs {
        rules.push(mark(
            pre,
            vec![
                "-A".to_string(),
                pre.to_string(),
                "-d".to_string(),
                cidr.clone(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                TUN_ROUTE_MARK.to_string(),
            ],
        ));
        rules.push(mark(
            pre,
            vec![
                "-A".to_string(),
                pre.to_string(),
                "-d".to_string(),
                cidr.clone(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    rules.push(local_ip_jump(family, "mangle", pre));
    rules.extend(user_rules_for(
        ctx.document,
        ctx.mode,
        family,
        Scope::Forward,
        "mangle",
        pre,
        false,
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
                TUN_BYPASS_MARK.to_string(),
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
            TUN_BYPASS_MARK.to_string(),
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
            TUN_ROUTE_MARK.to_string(),
        ],
    ));

    // --- BOX_TUN_BYPASS_OUT ---
    rules.push(core_owner_bypass_rule(family, "mangle", out));
    if !has_adb_default_rule(ctx.document) {
        rules.extend(adb_port_bypass_rules(family, "mangle", out, true, false));
    }
    for (proto, enabled_flag) in [("tcp", ctx.dns_hijack_tcp), ("udp", ctx.dns_hijack_udp)] {
        if enabled_flag && ctx.dns_hijack_mode != "disable" {
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
                    "-p".to_string(),
                    proto.to_string(),
                    "--dport".to_string(),
                    "53".to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    TUN_ROUTE_MARK.to_string(),
                ],
            ));
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
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
    if let Some(fake) = fake_ip {
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-d".to_string(),
                fake.to_string(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                TUN_ROUTE_MARK.to_string(),
            ],
        ));
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-d".to_string(),
                fake.to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    for cidr in force_cidrs {
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-d".to_string(),
                cidr.clone(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                TUN_ROUTE_MARK.to_string(),
            ],
        ));
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-d".to_string(),
                cidr.clone(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    if let Some(set) = cnip_set {
        for uid in &ctx.force_uids {
            rules.push(mark(
                out,
                vec![
                    "-A".to_string(),
                    out.to_string(),
                    "-m".to_string(),
                    "owner".to_string(),
                    "--uid-owner".to_string(),
                    uid.to_string(),
                    "-m".to_string(),
                    "set".to_string(),
                    "--match-set".to_string(),
                    set.to_string(),
                    "dst".to_string(),
                    "-j".to_string(),
                    "MARK".to_string(),
                    "--set-xmark".to_string(),
                    TUN_ROUTE_MARK.to_string(),
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
                    uid.to_string(),
                    "-m".to_string(),
                    "set".to_string(),
                    "--match-set".to_string(),
                    set.to_string(),
                    "dst".to_string(),
                    "-j".to_string(),
                    "RETURN".to_string(),
                ],
            ));
        }
    }
    rules.push(local_ip_jump(family, "mangle", out));
    rules.extend(user_rules_for(
        ctx.document,
        ctx.mode,
        family,
        Scope::Local,
        "mangle",
        out,
        true,
    ));
    if let Some(set) = cnip_set {
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-m".to_string(),
                "set".to_string(),
                "--match-set".to_string(),
                set.to_string(),
                "dst".to_string(),
                "-j".to_string(),
                "MARK".to_string(),
                "--set-xmark".to_string(),
                TUN_BYPASS_MARK.to_string(),
            ],
        ));
        rules.push(mark(
            out,
            vec![
                "-A".to_string(),
                out.to_string(),
                "-m".to_string(),
                "mark".to_string(),
                "--mark".to_string(),
                TUN_BYPASS_MARK.to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        ));
    }
    if ctx.proxy_mode == "blacklist" || ctx.proxy_mode == "black" {
        for uid in &ctx.proxy_uids {
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
        for gid in &ctx.proxy_gids {
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
    } else if (ctx.proxy_mode == "whitelist" || ctx.proxy_mode == "white")
        && (!ctx.proxy_uids.is_empty() || !ctx.proxy_gids.is_empty())
    {
        let mut effective_uids = BTreeSet::new();
        effective_uids.insert(0u32);
        effective_uids.insert(ANDROID_DNS_UID);
        for uid in &ctx.proxy_uids {
            effective_uids.insert(*uid);
        }
        for uid in &effective_uids {
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
                    TUN_ROUTE_MARK.to_string(),
                ],
            ));
        }
        for gid in &ctx.proxy_gids {
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
                    TUN_ROUTE_MARK.to_string(),
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
            TUN_ROUTE_MARK.to_string(),
        ],
    ));

    let ip_flag = if family == "ip6tables" { "-6" } else { "-4" };
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            ip_flag.to_string(),
            "rule".to_string(),
            "add".to_string(),
            "fwmark".to_string(),
            TUN_BYPASS_MARK.to_string(),
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
            ip_flag.to_string(),
            "rule".to_string(),
            "add".to_string(),
            "fwmark".to_string(),
            TUN_ROUTE_MARK.to_string(),
            "table".to_string(),
            TUN_ROUTE_TABLE.to_string(),
            "pref".to_string(),
            "99".to_string(),
        ],
    });
    rules.push(Rule {
        family: "ip",
        table: "",
        chain: "",
        args: vec![
            ip_flag.to_string(),
            "route".to_string(),
            "replace".to_string(),
            "default".to_string(),
            "dev".to_string(),
            tun_device.clone(),
            "table".to_string(),
            TUN_ROUTE_TABLE.to_string(),
        ],
    });
    rules
}

fn quic_block_rules(settings: &Value, ctx: &PlanContext<'_>) -> Vec<Rule> {
    if !settings
        .get("quic")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "disable")
    {
        return Vec::new();
    }
    if !ctx.proxy_udp {
        return Vec::new();
    }
    let mut families = vec!["iptables"];
    if ctx.ipv6 && ctx.caps.ip6tables {
        families.push("ip6tables");
    }
    let mut rules = Vec::new();
    for family in families {
        rules.push(Rule {
            family,
            table: "filter",
            chain: "BOX_QUIC_BLOCK",
            args: vec!["-N".to_string(), "BOX_QUIC_BLOCK".to_string()],
        });
        rules.push(Rule {
            family,
            table: "filter",
            chain: "OUTPUT",
            args: vec![
                "-A".to_string(),
                "OUTPUT".to_string(),
                "-j".to_string(),
                "BOX_QUIC_BLOCK".to_string(),
            ],
        });
        rules.push(Rule {
            family,
            table: "filter",
            chain: "FORWARD",
            args: vec![
                "-A".to_string(),
                "FORWARD".to_string(),
                "-j".to_string(),
                "BOX_QUIC_BLOCK".to_string(),
            ],
        });
        for port in [443, 80] {
            rules.push(Rule {
                family,
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
    }
    rules
}

fn nft_plan(plan: &mut Plan, ctx: &PlanContext<'_>) -> Result<Plan> {
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

    let is_tun = ctx.effective == "tun";
    let is_mixed = ctx.effective == "mixed";
    let tproxy = matches!(ctx.effective, "tproxy" | "enhance");
    let redirect = matches!(ctx.effective, "redirect" | "mixed" | "enhance");

    if is_tun || is_mixed {
        if !ctx.caps.tun {
            plan.warnings
                .push("TUN device is unavailable; cannot apply TUN mode".to_string());
            plan.degraded = true;
        }
        rules.push(nft_chain(
            "box_tun_forward",
            "filter hook forward priority filter",
        ));
        rules.push(nft_rule(
            "box_tun_forward",
            &format!("iifname \"{}\" accept", ctx.tun_device),
        ));
        rules.push(nft_rule(
            "box_tun_forward",
            &format!("oifname \"{}\" accept", ctx.tun_device),
        ));
    }

    if let (Some(port), Some((nat_udp, nat_tcp))) = (ctx.dns_port, should_emit_nat_dns(ctx)) {
        rules.extend(nft_dns_rules(port, nat_udp, nat_tcp));
    }

    if tproxy {
        let mangle_tcp = ctx.effective != "enhance" && ctx.proxy_tcp;
        let mangle_udp = ctx.proxy_udp;
        rules.push(nft_chain(
            "box_tproxy",
            "filter hook prerouting priority mangle",
        ));
        for cidr in [
            "0.0.0.0/8",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "224.0.0.0/4",
        ] {
            rules.push(nft_rule("box_tproxy", &format!("ip daddr {cidr} return")));
        }
        // NOTE: Do NOT emit `meta mark 0x01000000/0x01000000 return` before tproxy!
        // Also set `meta mark set 0x01000000` on tproxy rules so packets match policy routing.
        if mangle_tcp {
            rules.push(nft_rule(
                "box_tproxy",
                &format!(
                    "meta nfproto ipv4 meta l4proto tcp tproxy ip to :{} meta mark set 0x01000000",
                    ctx.tproxy_port
                ),
            ));
        }
        if mangle_udp {
            rules.push(nft_rule(
                "box_tproxy",
                &format!(
                    "meta nfproto ipv4 meta l4proto udp tproxy ip to :{} meta mark set 0x01000000",
                    ctx.tproxy_port
                ),
            ));
        }
        if ctx.ipv6 {
            for cidr in ["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"] {
                rules.push(nft_rule("box_tproxy", &format!("ip6 daddr {cidr} return")));
            }
            if mangle_tcp {
                rules.push(nft_rule(
                    "box_tproxy",
                    &format!(
                        "meta nfproto ipv6 meta l4proto tcp tproxy ip6 to [::]:{} meta mark set 0x01000000",
                        ctx.tproxy_port
                    ),
                ));
            }
            if mangle_udp {
                rules.push(nft_rule(
                    "box_tproxy",
                    &format!(
                        "meta nfproto ipv6 meta l4proto udp tproxy ip6 to [::]:{} meta mark set 0x01000000",
                        ctx.tproxy_port
                    ),
                ));
            }
        }
        // Local OUTPUT marking chain for nft tproxy
        rules.push(nft_chain(
            "box_tproxy_local",
            "route hook output priority mangle",
        ));
        rules.push(nft_rule(
            "box_tproxy_local",
            &format!("meta skuid {CORE_UID} meta skgid {CORE_GID} return"),
        ));
        for cidr in [
            "0.0.0.0/8",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "224.0.0.0/4",
        ] {
            rules.push(nft_rule(
                "box_tproxy_local",
                &format!("ip daddr {cidr} return"),
            ));
        }
        if ctx.ipv6 {
            for cidr in ["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"] {
                rules.push(nft_rule(
                    "box_tproxy_local",
                    &format!("ip6 daddr {cidr} return"),
                ));
            }
        }
        emit_nft_proxy_mode_rules(
            &mut rules,
            "box_tproxy_local",
            "meta mark set 0x01000000",
            mangle_tcp,
            mangle_udp,
            ctx,
        );

        rules.push(Rule {
            family: "ip",
            table: "",
            chain: "",
            args: vec![
                "-4".to_string(),
                "rule".to_string(),
                "add".to_string(),
                "fwmark".to_string(),
                TPROXY_MARK.to_string(),
                "table".to_string(),
                TPROXY_TABLE.to_string(),
                "pref".to_string(),
                TPROXY_PREF.to_string(),
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
                TPROXY_TABLE.to_string(),
            ],
        });
        if ctx.ipv6 {
            rules.push(Rule {
                family: "ip",
                table: "",
                chain: "",
                args: vec![
                    "-6".to_string(),
                    "rule".to_string(),
                    "add".to_string(),
                    "fwmark".to_string(),
                    TPROXY_MARK.to_string(),
                    "table".to_string(),
                    TPROXY_TABLE.to_string(),
                    "pref".to_string(),
                    TPROXY_PREF.to_string(),
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
                    TPROXY_TABLE.to_string(),
                ],
            });
        }
    }

    if redirect && ctx.proxy_tcp {
        rules.push(nft_chain(
            "box_redirect",
            "nat hook prerouting priority dstnat",
        ));
        for cidr in [
            "0.0.0.0/8",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "224.0.0.0/4",
        ] {
            rules.push(nft_rule("box_redirect", &format!("ip daddr {cidr} return")));
        }
        rules.push(nft_rule(
            "box_redirect",
            &format!("meta nfproto ipv4 meta l4proto tcp redirect to :{}", ctx.redir_port),
        ));
        if ctx.ipv6 {
            for cidr in ["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"] {
                rules.push(nft_rule("box_redirect", &format!("ip6 daddr {cidr} return")));
            }
            rules.push(nft_rule(
                "box_redirect",
                &format!("meta nfproto ipv6 meta l4proto tcp redirect to :{}", ctx.redir_port),
            ));
        }

        rules.push(nft_chain(
            "box_redirect_local",
            "nat hook output priority dstnat",
        ));
        rules.push(nft_rule(
            "box_redirect_local",
            &format!("meta skuid {CORE_UID} meta skgid {CORE_GID} return"),
        ));
        for cidr in [
            "0.0.0.0/8",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "224.0.0.0/4",
        ] {
            rules.push(nft_rule(
                "box_redirect_local",
                &format!("ip daddr {cidr} return"),
            ));
        }
        if ctx.ipv6 {
            for cidr in ["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"] {
                rules.push(nft_rule(
                    "box_redirect_local",
                    &format!("ip6 daddr {cidr} return"),
                ));
            }
        }
        emit_nft_proxy_mode_rules(
            &mut rules,
            "box_redirect_local",
            &format!("redirect to :{}", ctx.redir_port),
            true,
            false,
            ctx,
        );
    }

    plan.rules = rules;
    if !ctx.caps.nft {
        plan.degraded = true;
        plan.warnings.push("nft is unavailable".to_string());
    }
    Ok(plan.clone())
}

fn emit_nft_proxy_mode_rules(
    rules: &mut Vec<Rule>,
    chain: &str,
    action_expr: &str,
    tcp: bool,
    udp: bool,
    ctx: &PlanContext<'_>,
) {
    let protos: Vec<&str> = [("tcp", tcp), ("udp", udp)]
        .into_iter()
        .filter_map(|(p, on)| on.then_some(p))
        .collect();
    if ctx.proxy_mode == "blacklist" || ctx.proxy_mode == "black" {
        for uid in &ctx.proxy_uids {
            rules.push(nft_rule(chain, &format!("meta skuid {uid} return")));
        }
        for gid in &ctx.proxy_gids {
            rules.push(nft_rule(chain, &format!("meta skgid {gid} return")));
        }
        for proto in protos {
            rules.push(nft_rule(
                chain,
                &format!("meta l4proto {proto} {action_expr}"),
            ));
        }
    } else if (ctx.proxy_mode == "whitelist" || ctx.proxy_mode == "white")
        && (!ctx.proxy_uids.is_empty() || !ctx.proxy_gids.is_empty())
    {
        let mut effective_uids = BTreeSet::new();
        effective_uids.insert(0u32);
        effective_uids.insert(ANDROID_DNS_UID);
        for uid in &ctx.proxy_uids {
            effective_uids.insert(*uid);
        }
        for proto in protos {
            for uid in &effective_uids {
                rules.push(nft_rule(
                    chain,
                    &format!("meta skuid {uid} meta l4proto {proto} {action_expr}"),
                ));
            }
            for gid in &ctx.proxy_gids {
                rules.push(nft_rule(
                    chain,
                    &format!("meta skgid {gid} meta l4proto {proto} {action_expr}"),
                ));
            }
        }
    } else {
        for proto in protos {
            rules.push(nft_rule(
                chain,
                &format!("meta l4proto {proto} {action_expr}"),
            ));
        }
    }
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
    let mut rules = vec![
        nft_chain("box_dns", "nat hook output priority dstnat - 10"),
        nft_rule(
            "box_dns",
            &format!("meta skuid {CORE_UID} meta skgid {CORE_GID} return"),
        ),
        nft_chain("box_dns_pre", "nat hook prerouting priority dstnat - 10"),
    ];
    if udp {
        rules.push(nft_rule(
            "box_dns",
            &format!("udp dport 53 redirect to :{port}"),
        ));
        rules.push(nft_rule(
            "box_dns_pre",
            &format!("udp dport 53 redirect to :{port}"),
        ));
    }
    if tcp {
        rules.push(nft_rule(
            "box_dns",
            &format!("tcp dport 53 redirect to :{port}"),
        ));
        rules.push(nft_rule(
            "box_dns_pre",
            &format!("tcp dport 53 redirect to :{port}"),
        ));
    }
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
    let mut packages = BTreeMap::new();
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
    let mut uids = BTreeSet::new();
    for entry in apps {
        let entry = entry.trim();
        let (user, package) = match entry.split_once(':') {
            Some((user, package)) => (user.trim(), package.trim()),
            None => ("0", entry),
        };
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

fn u32_list(settings: &Value, key: &str) -> Vec<u32> {
    settings
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u32::try_from(value).ok())
                .collect::<Vec<_>>()
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
            "dns_hijack_mode": "tproxy",
            "mihomo_dns_forward": "enable",
            "dns_hijack_tcp": "true",
            "dns_hijack_udp": "true",
            "bypass_cn_ip": "false",
            "bypass_cn_ip_v4": "false",
            "bypass_cn_ip_v6": "false",
            "performance_mode": "false",
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
            tproxy_v4: true,
            tproxy_v6: true,
            socket_transparent: true,
        }
    }

    #[test]
    fn plan_validation_rejects_duplicate_chain_and_invalid_filter_prerouting() {
        let dup = super::Plan {
            requested_mode: "tproxy".to_string(),
            effective_mode: "tproxy".to_string(),
            degraded: false,
            rules: vec![
                Rule {
                    family: "iptables",
                    table: "mangle",
                    chain: "BOX_DUP",
                    args: vec!["-N".to_string(), "BOX_DUP".to_string()],
                },
                Rule {
                    family: "iptables",
                    table: "mangle",
                    chain: "BOX_DUP",
                    args: vec!["-N".to_string(), "BOX_DUP".to_string()],
                },
            ],
            warnings: Vec::new(),
        };
        assert!(dup.validate().is_err());

        let bad_filter = super::Plan {
            requested_mode: "tproxy".to_string(),
            effective_mode: "tproxy".to_string(),
            degraded: false,
            rules: vec![
                Rule {
                    family: "iptables",
                    table: "filter",
                    chain: "BOX_FILTER_PRE",
                    args: vec!["-N".to_string(), "BOX_FILTER_PRE".to_string()],
                },
                Rule {
                    family: "iptables",
                    table: "filter",
                    chain: "PREROUTING",
                    args: vec![
                        "-A".to_string(),
                        "PREROUTING".to_string(),
                        "-j".to_string(),
                        "BOX_FILTER_PRE".to_string(),
                    ],
                },
            ],
            warnings: Vec::new(),
        };
        assert!(bad_filter.validate().is_err());
    }

    #[test]
    fn tproxy_is_pure_mangle_without_breaking_mark_return_and_supports_ipv6() {
        let mut values = settings("tproxy");
        values["ipv6"] = json!("true");
        let plan = plan(&values, full()).expect("tproxy plan");
        plan.validate().expect("valid tproxy plan");
        // No mark 0x01000000 RETURN in BOX_TPROXY!
        assert!(!plan.rules.iter().any(|rule| {
            rule.chain == "BOX_TPROXY"
                && rule.args.iter().any(|a| a == "--mark")
                && rule.args.iter().any(|a| a == "RETURN")
        }));
        // Local TCP & UDP both use mangle BOX_TPROXY_LOCAL MARK (no BOX_LOCAL_REDIRECT in tproxy).
        assert!(!plan.rules.iter().any(|r| r.chain == "BOX_LOCAL_REDIRECT"));
        for fam in ["iptables", "ip6tables"] {
            for proto in ["tcp", "udp"] {
                assert!(
                    plan.rules.iter().any(|r| {
                        r.family == fam
                            && r.table == "mangle"
                            && r.chain == "BOX_TPROXY_LOCAL"
                            && r.args.windows(2).any(|w| w == ["-p", proto])
                            && r.args.iter().any(|a| a == "MARK")
                    }),
                    "missing local mangle {fam}/{proto}"
                );
            }
            // Core bypass checks BOTH uid 0 and gid 3005.
            assert!(plan.rules.iter().any(|r| {
                r.family == fam
                    && r.chain == "BOX_TPROXY_LOCAL"
                    && r.args.windows(2).any(|w| w == ["--uid-owner", "0"])
                    && r.args.windows(2).any(|w| w == ["--gid-owner", "3005"])
            }));
        }
    }

    #[test]
    fn enhance_keeps_tcp_out_of_mangle_and_uses_nat_redirect_for_tcp() {
        let values = settings("enhance");
        let plan = plan(&values, full()).expect("enhance plan");
        plan.validate().expect("enhance plan valid");
        // mangle BOX_TPROXY must NOT have TCP TPROXY rules in enhance mode.
        assert!(!plan.rules.iter().any(|r| {
            r.table == "mangle"
                && r.chain == "BOX_TPROXY"
                && r.args.windows(2).any(|w| w == ["-p", "tcp"])
                && r.args.iter().any(|a| a == "TPROXY")
        }));
        // mangle BOX_TPROXY_LOCAL must NOT mark TCP in enhance mode, only UDP.
        assert!(!plan.rules.iter().any(|r| {
            r.table == "mangle"
                && r.chain == "BOX_TPROXY_LOCAL"
                && r.args.windows(2).any(|w| w == ["-p", "tcp"])
                && r.args.iter().any(|a| a == "MARK")
        }));
        assert!(plan.rules.iter().any(|r| {
            r.table == "mangle"
                && r.chain == "BOX_TPROXY_LOCAL"
                && r.args.windows(2).any(|w| w == ["-p", "udp"])
                && r.args.iter().any(|a| a == "MARK")
        }));
        // nat BOX_REDIRECT must NOT contain -m socket --transparent.
        assert!(!plan.rules.iter().any(|r| {
            r.table == "nat" && r.args.iter().any(|a| a == "socket")
        }));
        assert!(plan.rules.iter().any(|r| r.chain == "BOX_LOCAL_REDIRECT"));
    }

    #[test]
    fn nat_dns_precedes_local_redirect_in_output_and_prerouting() {
        let mut values = settings("redirect");
        values["dns_hijack_mode"] = json!("redirect");
        let plan = super::plan_with_dns(&values, full(), Some(1053)).expect("redirect dns");
        let dns_out = plan
            .rules
            .iter()
            .position(|r| r.table == "nat" && r.chain == "OUTPUT" && r.args.contains(&"BOX_DNS".to_string()))
            .expect("BOX_DNS output jump");
        let redir_out = plan
            .rules
            .iter()
            .position(|r| {
                r.table == "nat"
                    && r.chain == "OUTPUT"
                    && r.args.contains(&"BOX_LOCAL_REDIRECT".to_string())
            })
            .expect("BOX_LOCAL_REDIRECT output jump");
        assert!(dns_out < redir_out, "BOX_DNS must precede BOX_LOCAL_REDIRECT");
    }

    #[test]
    fn reject_and_uid_and_port_range_user_rules_compile_validly() {
        let mut document = default_rule_document();
        document.rules.push(crate::network_rules::RuleSpec {
            id: "user-reject-fwd".to_string(),
            name: "reject fwd".to_string(),
            enabled: true,
            action: crate::network_rules::Action::Reject,
            scopes: vec![crate::network_rules::Scope::Forward],
            modes: vec![crate::network_rules::Mode::Tproxy],
            family: crate::network_rules::Family::Ipv4,
            protocol: crate::network_rules::Protocol::Tcp,
            match_fields: crate::network_rules::MatchFields {
                dst_ports: vec!["8000-9000".to_string()],
                ..Default::default()
            },
            origin: crate::network_rules::Origin::User,
        });
        document.rules.push(crate::network_rules::RuleSpec {
            id: "user-uid-bypass".to_string(),
            name: "uid bypass".to_string(),
            enabled: true,
            action: crate::network_rules::Action::Bypass,
            scopes: vec![crate::network_rules::Scope::Local],
            modes: vec![crate::network_rules::Mode::Tproxy],
            family: crate::network_rules::Family::Ipv4,
            protocol: crate::network_rules::Protocol::Any,
            match_fields: crate::network_rules::MatchFields {
                uids: vec![10234],
                ..Default::default()
            },
            origin: crate::network_rules::Origin::User,
        });
        let values = settings("tproxy");
        let plan =
            plan_with_dns_and_rules(&values, full(), Some(1053), Some(&document)).expect("plan");
        plan.validate().expect("valid plan");
        // Forward reject attaches to filter/FORWARD (BOX_FILTER_FWD), NOT filter/PREROUTING.
        assert!(plan.rules.iter().any(|r| {
            r.table == "filter"
                && r.chain == "FORWARD"
                && r.args.iter().any(|a| a == "BOX_FILTER_FWD")
        }));
        assert!(!plan
            .rules
            .iter()
            .any(|r| r.table == "filter" && r.chain == "PREROUTING"));
        // Port range converted from 8000-9000 to 8000:9000.
        assert!(plan.rules.iter().any(|r| {
            r.chain == "BOX_FILTER_FWD" && r.args.windows(2).any(|w| w == ["--dport", "8000:9000"])
        }));
        // UID rule actually emitted in BOX_TPROXY_LOCAL!
        assert!(plan.rules.iter().any(|r| {
            r.chain == "BOX_TPROXY_LOCAL"
                && r.args.windows(2).any(|w| w == ["--uid-owner", "10234"])
                && r.args.iter().any(|a| a == "RETURN")
        }));
    }

    #[test]
    fn tun_bypass_only_when_cnip_enabled_and_respects_custom_device_and_route_replace_rollback() {
        let mut values = settings("tun");
        values["__tun_device"] = json!("utun0");
        let plain = plan(&values, full()).expect("tun plain");
        plain.validate().expect("plain valid");
        assert!(plain.rules.iter().any(|r| {
            r.table == "filter"
                && r.chain == "FORWARD"
                && r.args.windows(2).any(|w| w == ["-i", "utun0"])
        }));
        assert!(!plain.rules.iter().any(|r| r.chain == "BOX_TUN_BYPASS_PRE"));

        values["bypass_cn_ip"] = json!("true");
        values["bypass_cn_ip_v4"] = json!("true");
        let cnip = plan(&values, full()).expect("tun cnip");
        cnip.validate().expect("cnip valid");
        assert!(cnip.rules.iter().any(|r| r.family == "ipset"));
        assert!(cnip.rules.iter().any(|r| r.chain == "BOX_TUN_BYPASS_PRE"));
        assert!(cnip.rules.iter().any(|r| r.chain == "BOX_TUN_BYPASS_OUT"));
        // Rollback must delete `ip -4 route replace default dev utun0 table 2025`!
        let rollback = super::rollback_rules(cnip.rules.iter().rev());
        assert!(rollback.iter().any(|r| {
            r.family == "ip"
                && r.args.windows(2).any(|w| w == ["route", "del"])
                && r.args.iter().any(|a| a == "utun0")
        }));
    }

    #[test]
    fn local_output_rules_follow_proxy_mode_and_whitelist_includes_root_and_dns_uid() {
        let mut black = settings("tproxy");
        black["proxy_mode"] = json!("blacklist");
        black["__proxy_uids"] = json!([10001]);
        let black_plan = plan(&black, full()).expect("blacklist");
        assert!(black_plan.rules.iter().any(|r| {
            r.chain == "BOX_TPROXY_LOCAL"
                && r.args.windows(2).any(|w| w == ["--uid-owner", "10001"])
                && r.args.iter().any(|a| a == "RETURN")
        }));

        let mut white = settings("tproxy");
        white["proxy_mode"] = json!("whitelist");
        white["__proxy_uids"] = json!([10002]);
        let white_plan = plan(&white, full()).expect("whitelist");
        for expected_uid in ["0", "1052", "10002"] {
            assert!(
                white_plan.rules.iter().any(|r| {
                    r.chain == "BOX_TPROXY_LOCAL"
                        && r.args.windows(2).any(|w| w == ["--uid-owner", expected_uid])
                        && r.args.iter().any(|a| a == "MARK")
                }),
                "whitelist missing uid {expected_uid}"
            );
        }
    }

    #[test]
    fn ap_list_and_mac_filter_dispatch_correctly() {
        let mut values = settings("tproxy");
        values["__ap_allow"] = json!(["wlan+"]);
        values["__ap_ignore"] = json!(["rmnet+"]);
        values["mac_filter"] = json!("true");
        values["mac_mode"] = json!("whitelist");
        values["macs_list"] = json!(["AA:BB:CC:DD:EE:FF"]);
        let plan = plan(&values, full()).expect("ap mac plan");
        plan.validate().expect("valid");
        assert!(plan.rules.iter().any(|r| {
            r.chain == "BOX_TPROXY"
                && r.args.windows(2).any(|w| w == ["-i", "rmnet+"])
                && r.args.iter().any(|a| a == "RETURN")
        }));
        assert!(plan.rules.iter().any(|r| {
            r.chain == "BOX_TPROXY"
                && r.args.windows(2).any(|w| w == ["-i", "wlan+"])
                && r.args.windows(2).any(|w| w == ["--mac-source", "aa:bb:cc:dd:ee:ff"])
                && r.args.iter().any(|a| a == "TPROXY")
        }));
    }

    #[test]
    fn performance_mode_adds_divert_and_conntrack_reply_bypass() {
        let mut values = settings("tproxy");
        values["performance_mode"] = json!("true");
        let plan = plan(&values, full()).expect("perf plan");
        plan.validate().expect("valid");
        assert!(plan.rules.iter().any(|r| r.chain == "DIVERT"));
        assert!(plan.rules.iter().any(|r| {
            r.chain == "BOX_TPROXY" && r.args.windows(2).any(|w| w == ["--ctdir", "REPLY"])
        }));
    }

    #[test]
    fn cnip_force_proxy_uids_and_local_cnip_do_not_drop_regular_local_proxy() {
        let list = "com.example.app 10123 0 /data/user/0/com.example.app\n";
        assert_eq!(
            super::parse_force_uids(&["com.example.app", " 10 : com.example.app "], list),
            vec![10123, 1_010_123]
        );
        let mut values = settings("tproxy");
        values["bypass_cn_ip"] = json!("true");
        values["bypass_cn_ip_v4"] = json!("true");
        let plan = plan(&values, full()).expect("cnip local");
        assert!(plan.rules.iter().any(|r| {
            r.chain == "BOX_TPROXY_LOCAL"
                && r.args.windows(2).any(|w| w == ["--match-set", "cnip"])
                && r.args.iter().any(|a| a == "RETURN")
        }));
    }

    #[test]
    fn nft_all_five_modes_create_table_and_fix_mixed_and_local_output() {
        let mut caps = full();
        caps.iptables = false;
        caps.nft = true;
        for mode in ["tun", "tproxy", "redirect", "mixed", "enhance"] {
            let mut values = settings(mode);
            values["ipv6"] = json!("true");
            values["dns_hijack_mode"] = json!("redirect");
            let plan = super::plan_with_dns(&values, caps, Some(1053)).expect(mode);
            assert_eq!(plan.rules[0].family, "nft", "{mode} first rule must be nft");
            assert!(
                plan.rules[0].args.windows(2).any(|w| w == ["add", "table"]),
                "{mode} must create inet box table first"
            );
            assert!(
                !plan.rules.iter().any(|r| {
                    r.args.iter().any(|a| a == "box_tproxy")
                        && r.args.iter().any(|a| a == "return")
                        && r.args.iter().any(|a| a == "0x01000000/0x01000000")
                }),
                "{mode} must not return on 0x01000000 mark before tproxy"
            );
            if mode == "mixed" {
                assert!(plan.rules.iter().any(|r| r.args.iter().any(|a| a == "box_tun_forward")));
                assert!(plan.rules.iter().any(|r| r.args.iter().any(|a| a == "box_redirect")));
                assert!(!plan.rules.iter().any(|r| r.args.iter().any(|a| a == "box_tproxy")));
            }
            if mode == "tproxy" {
                assert!(plan.rules.iter().any(|r| r.args.iter().any(|a| a == "box_tproxy_local")));
            }
        }
    }

    #[test]
    fn parse_ip_addr_output_extracts_non_tun_v4_and_global_v6() {
        let v4 = "1: lo: <LOOPBACK,UP>\n    inet 127.0.0.1/8 scope host lo\n2: wlan0: <UP>\n    inet 192.168.1.12/24 brd 192.168.1.255 scope global wlan0\n3: meta: <UP>\n    inet 198.18.0.1/30 scope global meta\n";
        assert_eq!(
            super::parse_ip_addr_output(v4, "meta", false),
            vec!["192.168.1.12/24".to_string()]
        );
        let v6 = "20: rmnet_data1@rmnet_ipa0: <UP>\n    inet6 2409:815a:ec81::1/64 scope global dynamic\n    inet6 fe80::1/64 scope link\n";
        assert_eq!(
            super::parse_ip_addr_output(v6, "meta", true),
            vec!["2409:815a:ec81::1/128".to_string()]
        );
    }

    #[test]
    fn apply_and_rollback_work() {
        let plan = plan(&settings("tproxy"), full()).expect("plan");
        let mut runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        apply(&plan, &mut runner).expect("apply");
        assert_eq!(runner.commands.len(), plan.rules.len());
        let mut failing = FakeRunner {
            commands: Vec::new(),
            fail_at: Some(3),
        };
        assert!(apply(&plan, &mut failing).is_err());
        let mut cleanup_runner = FakeRunner {
            commands: Vec::new(),
            fail_at: None,
        };
        super::cleanup(&plan, &mut cleanup_runner).expect("cleanup");
        assert!(cleanup_runner.commands.iter().any(|cmd| {
            cmd.windows(2).any(|w| w == ["rule", "del"]) && cmd.iter().any(|a| a == "2024")
        }));
    }
}
