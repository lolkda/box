use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;

pub const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Bypass,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Local,
    Forward,
    Dns,
    Filter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Tun,
    Tproxy,
    Redirect,
    Mixed,
    Enhance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Ipv4,
    Ipv6,
    Both,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Any,
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DstType {
    Local,
    Multicast,
    Broadcast,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Default,
    #[default]
    User,
}

fn all_modes() -> Vec<Mode> {
    vec![
        Mode::Tun,
        Mode::Tproxy,
        Mode::Redirect,
        Mode::Mixed,
        Mode::Enhance,
    ]
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchFields {
    #[serde(default)]
    pub dst_cidrs: Vec<String>,
    #[serde(default)]
    pub src_cidrs: Vec<String>,
    #[serde(default)]
    pub dst_ports: Vec<String>,
    #[serde(default)]
    pub src_ports: Vec<String>,
    #[serde(default)]
    pub src_ifaces: Vec<String>,
    #[serde(default)]
    pub dst_ifaces: Vec<String>,
    #[serde(default)]
    pub uids: Vec<u32>,
    #[serde(default)]
    pub gids: Vec<u32>,
    #[serde(default)]
    pub dst_types: Vec<DstType>,
}

impl MatchFields {
    pub fn is_empty(&self) -> bool {
        self.dst_cidrs.is_empty()
            && self.src_cidrs.is_empty()
            && self.dst_ports.is_empty()
            && self.src_ports.is_empty()
            && self.src_ifaces.is_empty()
            && self.dst_ifaces.is_empty()
            && self.uids.is_empty()
            && self.gids.is_empty()
            && self.dst_types.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSpec {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub action: Action,
    pub scopes: Vec<Scope>,
    #[serde(default = "all_modes")]
    pub modes: Vec<Mode>,
    pub family: Family,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default, rename = "match")]
    pub match_fields: MatchFields,
    #[serde(default)]
    pub origin: Origin,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub revision: u64,
    pub defaults_version: String,
    pub rules: Vec<RuleSpec>,
    #[serde(default)]
    pub tombstones: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub base_revision: Option<u64>,
    pub rules: Vec<RuleSpec>,
    #[serde(default)]
    pub tombstones: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diff {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub reordered: bool,
    pub enabled: usize,
    pub disabled: usize,
    pub added_ids: Vec<String>,
    pub removed_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub document: Document,
    pub diff: Diff,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GetResult {
    pub schema_version: u32,
    pub revision: u64,
    pub defaults_version: String,
    pub rules: Vec<RuleSpec>,
    pub tombstones: Vec<String>,
    pub system: Vec<Value>,
    pub warnings: Vec<String>,
}

pub fn validate_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub fn validate(document: &mut Document) -> Result<()> {
    if document.schema_version != SCHEMA_VERSION {
        return Err(BoxError::new(
            ErrorCode::VersionMismatch,
            format!(
                "unsupported network rules schema: {}",
                document.schema_version
            ),
        ));
    }
    let mut ids = BTreeSet::new();
    for rule in &mut document.rules {
        if !validate_id(&rule.id) {
            return Err(BoxError::validation(format!(
                "invalid rule id: {}",
                rule.id
            )));
        }
        if !ids.insert(rule.id.clone()) {
            return Err(BoxError::validation(format!(
                "duplicate rule id: {}",
                rule.id
            )));
        }
        validate_rule(rule)?;
    }
    let mut seen = BTreeSet::new();
    document.tombstones.retain(|id| {
        if !validate_id(id) || !seen.insert(id.clone()) {
            return false;
        }
        true
    });
    Ok(())
}

pub fn validate_rule(rule: &mut RuleSpec) -> Result<()> {
    rule.id = rule.id.trim().to_string();
    rule.name = rule.name.trim().to_string();
    if !validate_id(&rule.id) {
        return Err(BoxError::validation(format!(
            "invalid rule id: {}",
            rule.id
        )));
    }
    if rule.name.is_empty() || rule.name.chars().count() > 80 {
        return Err(BoxError::validation(format!(
            "rule {} name must be 1-80 characters",
            rule.id
        )));
    }
    if rule.scopes.is_empty() {
        return Err(BoxError::validation(format!(
            "rule {} must select at least one scope",
            rule.id
        )));
    }
    if rule.modes.is_empty() {
        return Err(BoxError::validation(format!(
            "rule {} must select at least one mode",
            rule.id
        )));
    }
    let mut scopes = BTreeSet::new();
    for scope in &rule.scopes {
        let key = format!("{scope:?}");
        if !scopes.insert(key) {
            return Err(BoxError::validation(format!(
                "rule {} has duplicate scope",
                rule.id
            )));
        }
    }
    if rule.action == Action::Reject && rule.scopes.contains(&Scope::Dns) {
        return Err(BoxError::validation(format!(
            "rule {} cannot reject DNS with this schema",
            rule.id
        )));
    }
    if rule.action == Action::Bypass && rule.scopes.contains(&Scope::Filter) {
        return Err(BoxError::validation(format!(
            "rule {} cannot bypass through the filter scope",
            rule.id
        )));
    }
    if rule.match_fields.is_empty() {
        return Err(BoxError::validation(format!(
            "rule {} has an empty match",
            rule.id
        )));
    }
    validate_family(&rule.id, rule.family, &rule.match_fields)?;
    for cidr in rule
        .match_fields
        .dst_cidrs
        .iter()
        .chain(rule.match_fields.src_cidrs.iter())
    {
        validate_cidr(&rule.id, cidr)?;
    }
    for port in rule
        .match_fields
        .dst_ports
        .iter()
        .chain(rule.match_fields.src_ports.iter())
    {
        validate_port(&rule.id, port)?;
    }
    for iface in rule
        .match_fields
        .src_ifaces
        .iter()
        .chain(rule.match_fields.dst_ifaces.iter())
    {
        validate_iface(&rule.id, iface)?;
    }
    if rule.match_fields.uids.contains(&0) {
        return Err(BoxError::validation(format!(
            "rule {} cannot match root uid 0; root is protected",
            rule.id
        )));
    }
    Ok(())
}

fn validate_family(id: &str, family: Family, fields: &MatchFields) -> Result<()> {
    if family == Family::Both {
        return Ok(());
    }
    for cidr in fields.dst_cidrs.iter().chain(fields.src_cidrs.iter()) {
        let address = cidr.split('/').next().unwrap_or_default();
        let parsed = address
            .parse::<std::net::IpAddr>()
            .map_err(|_| BoxError::validation(format!("rule {id} has invalid CIDR: {cidr}")))?;
        if family == Family::Ipv4 && !parsed.is_ipv4() {
            return Err(BoxError::validation(format!(
                "rule {id} mixes IPv6 CIDR with IPv4 family"
            )));
        }
        if family == Family::Ipv6 && !parsed.is_ipv6() {
            return Err(BoxError::validation(format!(
                "rule {id} mixes IPv4 CIDR with IPv6 family"
            )));
        }
    }
    Ok(())
}

fn validate_cidr(id: &str, value: &str) -> Result<()> {
    let (address, prefix) = value.split_once('/').ok_or_else(|| {
        BoxError::validation(format!("rule {id} CIDR must include a prefix: {value}"))
    })?;
    let address = address
        .parse::<std::net::IpAddr>()
        .map_err(|_| BoxError::validation(format!("rule {id} invalid CIDR address: {value}")))?;
    let prefix = prefix
        .parse::<u8>()
        .map_err(|_| BoxError::validation(format!("rule {id} invalid CIDR prefix: {value}")))?;
    let maximum = if address.is_ipv4() { 32 } else { 128 };
    if prefix > maximum {
        return Err(BoxError::validation(format!(
            "rule {id} CIDR prefix out of range: {value}"
        )));
    }
    Ok(())
}

fn validate_port(id: &str, value: &str) -> Result<()> {
    let (start, end) = match value.split_once('-') {
        Some((start, end)) => (start, end),
        None => (value, value),
    };
    let parse = |text: &str| {
        text.parse::<u16>()
            .map_err(|_| BoxError::validation(format!("rule {id} invalid port: {value}")))
    };
    let start = parse(start)?;
    let end = parse(end)?;
    if start == 0 || end == 0 || start > end {
        return Err(BoxError::validation(format!(
            "rule {id} invalid port range: {value}"
        )));
    }
    Ok(())
}

fn validate_iface(id: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 32
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'+' | b'@')
        })
    {
        return Err(BoxError::validation(format!(
            "rule {id} invalid interface: {value}"
        )));
    }
    Ok(())
}

pub fn diff(old: &Document, new: &Document) -> Diff {
    let old_map: BTreeMap<&str, &RuleSpec> = old
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect();
    let new_map: BTreeMap<&str, &RuleSpec> = new
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect();
    let mut added_ids = Vec::new();
    let mut removed_ids = Vec::new();
    let mut updated = 0;
    let mut enabled = 0;
    let mut disabled = 0;
    for (id, rule) in &new_map {
        match old_map.get(id) {
            None => added_ids.push((*id).to_string()),
            Some(old_rule) => {
                if *rule != *old_rule {
                    updated += 1;
                    if !old_rule.enabled && rule.enabled {
                        enabled += 1;
                    }
                    if old_rule.enabled && !rule.enabled {
                        disabled += 1;
                    }
                }
            }
        }
    }
    for id in old_map.keys() {
        if !new_map.contains_key(id) {
            removed_ids.push((*id).to_string());
        }
    }
    let old_common = old
        .rules
        .iter()
        .filter(|rule| new_map.contains_key(rule.id.as_str()))
        .map(|rule| rule.id.as_str())
        .collect::<Vec<_>>();
    let new_common = new
        .rules
        .iter()
        .filter(|rule| old_map.contains_key(rule.id.as_str()))
        .map(|rule| rule.id.as_str())
        .collect::<Vec<_>>();
    Diff {
        added: added_ids.len(),
        updated,
        removed: removed_ids.len(),
        reordered: old_common != new_common,
        enabled,
        disabled,
        added_ids,
        removed_ids,
    }
}

pub fn get(paths: &Paths) -> Result<GetResult> {
    let mut warnings = Vec::new();
    let mut document = load(paths, &mut warnings)?;
    validate(&mut document)?;
    let system = crate::network::system_protection_summary();
    Ok(GetResult {
        schema_version: document.schema_version,
        revision: document.revision,
        defaults_version: document.defaults_version,
        rules: document.rules,
        tombstones: document.tombstones,
        system,
        warnings,
    })
}

pub fn load_for_plan(paths: &Paths) -> Result<Document> {
    let mut warnings = Vec::new();
    load(paths, &mut warnings)
}

pub fn preview(paths: &Paths, mutation: Mutation) -> Result<Preview> {
    let mut warnings = Vec::new();
    let old = load(paths, &mut warnings)?;
    let mut new = prepare_mutation(&old, mutation)?;
    validate(&mut new)?;
    let diff = diff(&old, &new);
    if !new.rules.iter().any(|rule| rule.enabled) {
        warnings.push("所有用户规则都已禁用，仅保留系统保护".to_string());
    }
    for rule in &new.rules {
        if rule.enabled
            && rule
                .match_fields
                .dst_cidrs
                .iter()
                .any(|cidr| cidr == "0.0.0.0/0" || cidr == "::/0")
        {
            warnings.push(format!(
                "规则 {} 匹配全部地址，请确认这是预期行为",
                rule.name
            ));
        }
    }
    Ok(Preview {
        document: new,
        diff,
        warnings,
        plan: None,
    })
}

pub fn apply(paths: &Paths, mutation: Mutation) -> Result<Document> {
    let mut warnings = Vec::new();
    let old = load(paths, &mut warnings)?;
    let mut new = prepare_mutation(&old, mutation)?;
    validate(&mut new)?;
    new.revision = old.revision.saturating_add(1);
    save(paths, &new)?;
    Ok(new)
}

pub fn reset_defaults(paths: &Paths, all: bool) -> Result<Document> {
    let mut warnings = Vec::new();
    let old = load(paths, &mut warnings)?;
    let defaults = read_defaults(paths)?;
    let mut document = old.clone();
    document.revision = old.revision.saturating_add(1);
    if all {
        document.rules.clear();
        document.tombstones = defaults.rules.iter().map(|rule| rule.id.clone()).collect();
    } else {
        let custom = old
            .rules
            .iter()
            .filter(|rule| rule.origin == Origin::User)
            .cloned()
            .collect::<Vec<_>>();
        document.rules = defaults
            .rules
            .iter()
            .map(|rule| {
                let mut rule = rule.clone();
                rule.origin = Origin::Default;
                rule
            })
            .chain(custom)
            .collect();
        document.tombstones.clear();
        document.defaults_version = defaults.defaults_version;
    }
    validate(&mut document)?;
    save(paths, &document)?;
    Ok(document)
}

fn prepare_mutation(old: &Document, mutation: Mutation) -> Result<Document> {
    if let Some(base) = mutation.base_revision {
        if base != old.revision {
            return Err(BoxError::new(
                ErrorCode::Conflict,
                format!(
                    "network rules revision conflict: base {base}, current {}",
                    old.revision
                ),
            ));
        }
    }
    let mut rules = mutation.rules;
    let mut ids = BTreeSet::new();
    for rule in &mut rules {
        rule.id = rule.id.trim().to_string();
        rule.name = rule.name.trim().to_string();
        if !validate_id(&rule.id) {
            return Err(BoxError::validation(format!(
                "invalid rule id: {}",
                rule.id
            )));
        }
        if !ids.insert(rule.id.clone()) {
            return Err(BoxError::validation(format!(
                "duplicate rule id: {}",
                rule.id
            )));
        }
        if rule.id.starts_with("default-") {
            rule.origin = Origin::Default;
        }
    }
    let mut tombstones = mutation.tombstones;
    let current_defaults = old
        .rules
        .iter()
        .filter(|rule| rule.origin == Origin::Default)
        .map(|rule| rule.id.clone())
        .collect::<BTreeSet<_>>();
    let next_ids = rules
        .iter()
        .map(|rule| rule.id.clone())
        .collect::<BTreeSet<_>>();
    for id in current_defaults {
        if !next_ids.contains(&id) {
            tombstones.push(id);
        }
    }
    tombstones.sort();
    tombstones.dedup();
    Ok(Document {
        schema_version: SCHEMA_VERSION,
        revision: old.revision,
        defaults_version: old.defaults_version.clone(),
        rules,
        tombstones,
    })
}

fn load(paths: &Paths, warnings: &mut Vec<String>) -> Result<Document> {
    let defaults = read_defaults(paths).ok();
    if defaults.is_none() {
        warnings.push(format!(
            "默认网络规则文件不存在或无法读取: {}",
            paths.network_rules_default.display()
        ));
    }
    let mut document = match std::fs::read(&paths.network_rules) {
        Ok(bytes) => match serde_json::from_slice::<Document>(&bytes) {
            Ok(document) => document,
            Err(error) => {
                return Err(BoxError::new(
                    ErrorCode::Content,
                    format!("cannot parse {}", paths.network_rules.display()),
                )
                .with_detail(error.to_string()))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Document {
            schema_version: SCHEMA_VERSION,
            revision: 1,
            defaults_version: String::new(),
            rules: Vec::new(),
            tombstones: Vec::new(),
        },
        Err(error) => return Err(BoxError::from(error)),
    };
    validate(&mut document)?;
    let before = document.clone();
    if let Some(defaults) = defaults {
        merge_defaults(&mut document, &defaults)?;
    }
    if document != before {
        document.revision = before.revision.saturating_add(1);
        save(paths, &document)?;
    }
    Ok(document)
}

fn merge_defaults(document: &mut Document, defaults: &Document) -> Result<()> {
    let existing = document
        .rules
        .iter()
        .map(|rule| rule.id.clone())
        .collect::<BTreeSet<_>>();
    let tombstones = document.tombstones.iter().cloned().collect::<BTreeSet<_>>();
    for rule in &defaults.rules {
        if existing.contains(&rule.id) || tombstones.contains(&rule.id) {
            continue;
        }
        let mut rule = rule.clone();
        rule.origin = Origin::Default;
        document.rules.push(rule);
    }
    document.defaults_version = defaults.defaults_version.clone();
    Ok(())
}

fn read_defaults(paths: &Paths) -> Result<Document> {
    let bytes = std::fs::read(&paths.network_rules_default).map_err(|error| {
        BoxError::new(
            ErrorCode::NotFound,
            format!("cannot read {}", paths.network_rules_default.display()),
        )
        .with_detail(error.to_string())
    })?;
    let document = serde_json::from_slice::<Document>(&bytes).map_err(|error| {
        BoxError::new(
            ErrorCode::Content,
            format!("cannot parse {}", paths.network_rules_default.display()),
        )
        .with_detail(error.to_string())
    })?;
    Ok(document)
}

pub fn save_document(paths: &Paths, document: &Document) -> Result<()> {
    save(paths, document)
}

fn save(paths: &Paths, document: &Document) -> Result<()> {
    let parent = paths
        .network_rules
        .parent()
        .ok_or_else(|| BoxError::internal("network rules path has no parent"))?;
    util::ensure_dir(parent, true)?;
    let text = serde_json::to_string_pretty(document).map_err(|error| {
        BoxError::internal("cannot serialize network rules").with_detail(error.to_string())
    })?;
    util::atomic_write(&paths.network_rules, text.as_bytes(), 0o600)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "boxctl-network-rules-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        let paths = Paths::new(root);
        std::fs::create_dir_all(paths.network_rules.parent().expect("parent")).expect("state");
        std::fs::write(
            &paths.network_rules_default,
            r#"{"schema_version":1,"revision":0,"defaults_version":"test","rules":[{"id":"default-lan","name":"LAN","enabled":true,"action":"bypass","scopes":["local","forward"],"modes":["tun","tproxy","redirect","mixed","enhance"],"family":"ipv4","protocol":"any","match":{"dst_cidrs":["192.168.0.0/16"]},"origin":"default"}],"tombstones":[]}"#,
        )
        .expect("default");
        paths
    }

    #[test]
    fn packaged_defaults_are_valid() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../../box/network-rules.default.json"))
                .expect("default json");
        validate(&mut document).expect("default rules validate");
        assert!(document
            .rules
            .iter()
            .any(|rule| rule.id == "default-private-v4"));
        assert!(document
            .rules
            .iter()
            .any(|rule| rule.id == "default-discovery"));
    }

    #[test]
    fn seeds_defaults_and_persists_revision() {
        let paths = temp_paths("seed");
        let first = get(&paths).expect("get");
        assert_eq!(first.rules.len(), 1);
        assert!(paths.network_rules.is_file());
        let second = get(&paths).expect("get");
        assert_eq!(second.revision, first.revision);
    }

    #[test]
    fn deleting_default_records_tombstone() {
        let paths = temp_paths("tombstone");
        let current = get(&paths).expect("get");
        let mutation = Mutation {
            base_revision: Some(current.revision),
            rules: Vec::new(),
            tombstones: Vec::new(),
        };
        let applied = apply(&paths, mutation).expect("apply");
        assert!(applied.rules.is_empty());
        assert!(applied.tombstones.contains(&"default-lan".to_string()));
        let reloaded = get(&paths).expect("get");
        assert!(reloaded.rules.is_empty());
    }

    #[test]
    fn rejects_revision_conflict() {
        let paths = temp_paths("conflict");
        let current = get(&paths).expect("get");
        let mutation = Mutation {
            base_revision: Some(current.revision + 1),
            rules: current.rules,
            tombstones: current.tombstones,
        };
        let error = apply(&paths, mutation).expect_err("conflict");
        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn validates_ports_and_family() {
        let mut rule = RuleSpec {
            id: "custom".to_string(),
            name: "custom".to_string(),
            enabled: true,
            action: Action::Bypass,
            scopes: vec![Scope::Local],
            modes: all_modes(),
            family: Family::Ipv4,
            protocol: Protocol::Udp,
            match_fields: MatchFields {
                dst_ports: vec!["53".to_string()],
                ..MatchFields::default()
            },
            origin: Origin::User,
        };
        validate_rule(&mut rule).expect("valid");
        rule.match_fields.dst_ports = vec!["0".to_string()];
        assert_eq!(
            validate_rule(&mut rule).expect_err("invalid").code,
            ErrorCode::Validation
        );
    }
}
