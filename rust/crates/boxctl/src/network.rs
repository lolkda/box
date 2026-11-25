use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::error::{BoxError, ErrorCode, Result};
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
                    plan.rules.extend(dns_rules(port, hijack_udp, hijack_tcp));
                }
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
    if matches!(effective, "tproxy" | "mixed" | "enhance") {
        plan.rules.extend(tproxy_rules(
            "iptables",
            tproxy_port,
            proxy_tcp,
            proxy_udp,
            cnip_ipset.then_some("cnip"),
            &force_v4,
        ));
        if ipv6 && caps.ip6tables && caps.ip6tables_mangle {
            plan.rules.extend(tproxy_rules(
                "ip6tables",
                tproxy_port,
                proxy_tcp,
                proxy_udp,
                cnip_ipset.then_some("cnip6"),
                &force_v6,
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
        ));
        if ipv6 && caps.ip6tables && caps.ip6tables_nat {
            plan.rules.extend(redirect_rules(
                "ip6tables",
                redir_port,
                proxy_tcp,
                cnip_ipset.then_some("cnip6"),
                &force_v6,
            ));
        } else if ipv6 && caps.ip6tables && !caps.ip6tables_nat {
            plan.warnings.push(
                "ip6tables nat table is unavailable; IPv6 redirect rules are skipped".to_string(),
            );
            plan.degraded = true;
        }
    }
    if !force_uids.is_empty() && cnip_ipset {
        if matches!(effective, "tproxy" | "mixed" | "enhance") {
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
            "meta mark 0x10000000/0x10000000 return",
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
                "0x10000000/0x10000000".to_string(),
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
                    "0x10000000/0x10000000".to_string(),
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

fn dns_rules(port: u16, udp: bool, tcp: bool) -> Vec<Rule> {
    let mut rules = vec![Rule {
        family: "iptables",
        table: "nat",
        chain: "BOX_DNS",
        args: vec!["-N".to_string(), "BOX_DNS".to_string()],
    }];
    rules.push(Rule {
        family: "iptables",
        table: "nat",
        chain: "BOX_DNS",
        args: vec![
            "-A".to_string(),
            "BOX_DNS".to_string(),
            "-m".to_string(),
            "owner".to_string(),
            "--uid-owner".to_string(),
            "0".to_string(),
            "-j".to_string(),
            "RETURN".to_string(),
        ],
    });
    if udp {
        rules.push(Rule {
            family: "iptables",
            table: "nat",
            chain: "BOX_DNS",
            args: vec![
                "-A".to_string(),
                "BOX_DNS".to_string(),
                "-p".to_string(),
                "udp".to_string(),
                "--dport".to_string(),
                "53".to_string(),
                "-j".to_string(),
                "REDIRECT".to_string(),
                "--to-ports".to_string(),
                port.to_string(),
            ],
        });
    }
    if tcp {
        rules.push(Rule {
            family: "iptables",
            table: "nat",
            chain: "BOX_DNS",
            args: vec![
                "-A".to_string(),
                "BOX_DNS".to_string(),
                "-p".to_string(),
                "tcp".to_string(),
                "--dport".to_string(),
                "53".to_string(),
                "-j".to_string(),
                "REDIRECT".to_string(),
                "--to-ports".to_string(),
                port.to_string(),
            ],
        });
    }
    rules.push(Rule {
        family: "iptables",
        table: "nat",
        chain: "OUTPUT",
        args: vec![
            "-A".to_string(),
            "OUTPUT".to_string(),
            "-j".to_string(),
            "BOX_DNS".to_string(),
        ],
    });
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
                    "0x10000000/0x10000000".to_string(),
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

fn tproxy_rules(
    family: &'static str,
    port: u16,
    proxy_tcp: bool,
    proxy_udp: bool,
    cnip_set: Option<&'static str>,
    force_cidrs: &[String],
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
                    "0x10000000/0x10000000".to_string(),
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
                    "0x10000000/0x10000000".to_string(),
                ],
            });
        }
    }
    let local_networks: &[&str] = if family == "ip6tables" {
        &["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"]
    } else {
        &[
            "0.0.0.0/8",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "224.0.0.0/4",
        ]
    };
    for network in local_networks {
        rules.push(Rule {
            family,
            table: "mangle",
            chain: "BOX_TPROXY",
            args: vec![
                "-A".to_string(),
                "BOX_TPROXY".to_string(),
                "-d".to_string(),
                (*network).to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
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
            "0x10000000/0x10000000".to_string(),
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
                "0x10000000/0x10000000".to_string(),
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
                "0x10000000/0x10000000".to_string(),
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
            "0x10000000/0x10000000".to_string(),
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
    let local_networks: &[&str] = if family == "ip6tables" {
        &["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"]
    } else {
        &[
            "0.0.0.0/8",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "224.0.0.0/4",
        ]
    };
    for network in local_networks {
        rules.push(Rule {
            family,
            table: "nat",
            chain: "BOX_REDIRECT",
            args: vec![
                "-A".to_string(),
                "BOX_REDIRECT".to_string(),
                "-d".to_string(),
                (*network).to_string(),
                "-j".to_string(),
                "RETURN".to_string(),
            ],
        });
    }
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
    use super::{apply, plan, Capabilities, Rule, Runner};
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
    fn tun_has_no_iptables_rules() {
        let plan = plan(&settings("tun"), full()).expect("plan");
        assert!(plan.rules.is_empty());
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
        assert!(mixed.rules.len() > tproxy.rules.len());
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
        let mut values = settings("mixed");
        values["force_proxy_cidrs"] = serde_json::json!(["203.0.113.0/24"]);
        let plan = plan(&values, full()).expect("force cidrs");
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
        assert!(plan.rules.iter().any(|rule| {
            rule.chain == "BOX_REDIRECT"
                && rule.args.iter().any(|arg| arg == "203.0.113.0/24")
                && rule.args.iter().any(|arg| arg == "REDIRECT")
        }));
    }

    #[test]
    fn cnip_ipset_bypass_rules_are_generated() {
        let mut caps = full();
        caps.ipset = true;
        let mut values = settings("mixed");
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
        assert!(bypass_count >= 2, "both tproxy and redirect need bypass");
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
