use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::config;
use crate::download::{self, Policy};
use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;
use crate::yaml;

const ENTRY_MODE: u32 = 0o600;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddParams {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditParams {
    pub id: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UseParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub reload: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlParams {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetUrlParams {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchParams {
    #[serde(default)]
    pub id: Option<String>,
}

pub fn list(paths: &Paths) -> Result<Value> {
    ensure_layout(paths)?;
    let active = active_id(paths);
    let mut items = Vec::new();
    for id in ids(paths)? {
        let item = item(paths, &id, active.as_deref() == Some(id.as_str()))?;
        items.push(item);
    }
    Ok(json!({"supported": true, "items": items}))
}

pub fn get(paths: &Paths) -> Result<Value> {
    ensure_layout(paths)?;
    let Some(id) = selected_id(paths)? else {
        return Ok(json!({
            "supported": true,
            "configured": false,
            "id": "",
            "name": "",
            "url_masked": "",
            "updated_at": "",
            "counts": zero_counts().json()
        }));
    };
    let is_active = active_id(paths).as_deref() == Some(id.as_str());
    let item = item(paths, &id, is_active)?;
    Ok(json!({
        "supported": true,
        "configured": true,
        "id": item["id"],
        "name": item["name"],
        "url_masked": item["url_masked"],
        "updated_at": item["applied_at"],
        "counts": item["counts"]
    }))
}

pub fn add(paths: &Paths, params: AddParams) -> Result<Value> {
    ensure_layout(paths)?;
    validate_name(&params.name)?;
    validate_url(&params.url)?;
    if ids(paths)?
        .iter()
        .any(|id| read_entry(paths, id, "name").as_deref() == Some(params.name.as_str()))
    {
        return Err(BoxError::validation(format!(
            "rule profile name already exists: {}",
            params.name
        )));
    }
    let id = unique_id(paths, &params.name)?;
    write_entry(paths, &id, "name", &params.name)?;
    if let Err(error) = write_entry(paths, &id, "url", &params.url) {
        let _ = remove_entry_files(paths, &id);
        return Err(error);
    }
    item(paths, &id, active_id(paths).as_deref() == Some(id.as_str()))
}

pub fn edit(paths: &Paths, params: EditParams) -> Result<Value> {
    ensure_layout(paths)?;
    if !entry_exists(paths, &params.id) {
        return Err(not_found(&params.id));
    }
    validate_name(&params.name)?;
    validate_url(&params.url)?;
    for id in ids(paths)? {
        if id == params.id {
            continue;
        }
        if read_entry(paths, &id, "name").as_deref() == Some(params.name.as_str()) {
            return Err(BoxError::validation(format!(
                "rule profile name already exists: {}",
                params.name
            )));
        }
    }
    let old_name = read_entry(paths, &params.id, "name").unwrap_or_else(|| params.id.clone());
    let old_url = read_entry(paths, &params.id, "url").unwrap_or_default();
    write_entry(paths, &params.id, "name", &params.name)?;
    if let Err(error) = write_entry(paths, &params.id, "url", &params.url) {
        let _ = write_entry(paths, &params.id, "name", &old_name);
        return Err(error);
    }
    if old_url != params.url {
        for suffix in ["pending.yaml", "pending.yaml.bak", "fetched_at", "counts"] {
            let _ = util::remove_file_if_exists(&entry_path(paths, &params.id, suffix));
        }
        if active_id(paths).as_deref() == Some(params.id.as_str()) {
            let _ = util::remove_file_if_exists(&paths.rule_profile_active);
            let _ = util::remove_file_if_exists(&entry_path(paths, &params.id, "applied_at"));
        }
    }
    item(
        paths,
        &params.id,
        active_id(paths).as_deref() == Some(params.id.as_str()),
    )
}

pub fn remove(paths: &Paths, params: IdParams) -> Result<Value> {
    ensure_layout(paths)?;
    if !entry_exists(paths, &params.id) {
        return Err(not_found(&params.id));
    }
    if active_id(paths).as_deref() == Some(params.id.as_str()) {
        return Err(BoxError::validation(
            "active rule profile cannot be removed; deactivate it first",
        ));
    }
    remove_entry_files(paths, &params.id)?;
    Ok(json!({"removed": true, "id": params.id}))
}

pub fn deactivate(paths: &Paths) -> Result<Value> {
    ensure_layout(paths)?;
    util::remove_file_if_exists(&paths.rule_profile_active)?;
    Ok(json!({"active": false}))
}

pub fn fetch(paths: &Paths, params: FetchParams, policy: &Policy) -> Result<Value> {
    ensure_layout(paths)?;
    let id = params
        .id
        .or_else(|| selected_id(paths).ok().flatten())
        .ok_or_else(|| BoxError::new(ErrorCode::NotFound, "no rule profile is configured"))?;
    if !entry_exists(paths, &id) {
        return Err(not_found(&id));
    }
    let Some(url) = read_entry(paths, &id, "url").filter(|value| !value.is_empty()) else {
        return Err(BoxError::validation("rule profile URL is empty"));
    };
    let pending = entry_path(paths, &id, "pending.yaml");
    let temp = entry_path(
        paths,
        &id,
        &format!("pending.yaml.tmp.{}", std::process::id()),
    );
    let _ = util::remove_file_if_exists(&temp);
    let report = match download::download_to_file(&temp, &url, policy, None) {
        Ok(report) => report,
        Err(error) => {
            let _ = util::remove_file_if_exists(&temp);
            return Err(error);
        }
    };
    let mut map = match yaml::read_map(&temp) {
        Ok(map) => map,
        Err(error) => {
            let _ = util::remove_file_if_exists(&temp);
            return Err(error);
        }
    };
    // Legacy box.tool:2274 requires all three top-level keys to exist in a rule profile template.
    for required_key in ["proxy-groups", "rule-providers", "rules"] {
        if !map.contains_key(YamlValue::String(required_key.to_string())) {
            let _ = util::remove_file_if_exists(&temp);
            return Err(BoxError::validation(format!(
                "rule profile template is missing required section: {required_key}"
            )));
        }
    }
    let groups = section_json(&map, "proxy-groups", json!([]))?;
    let mut providers = section_json(&map, "rule-providers", json!({}))?;
    // Merge DNS-referenced rule-providers from existing config into pending snapshot so
    // preview counts and validation match what will be applied (legacy box.tool:3219-3298).
    preserve_dns_providers_in_json(paths, &mut providers);
    let rules = section_json(&map, "rules", json!([]))?;
    let data = json!({
        "proxy-groups": groups,
        "rule-providers": providers,
        "rules": rules
    });
    if let Err(error) = config::validate_sections(&data) {
        let _ = util::remove_file_if_exists(&temp);
        return Err(error);
    }
    // Persist merged rule-providers into pending.yaml so counts and apply stay consistent.
    if let Ok(providers_yaml) = serde_yaml::to_value(&data["rule-providers"]) {
        map.insert(
            YamlValue::String("rule-providers".to_string()),
            providers_yaml,
        );
        if let Ok(serialized) = serde_yaml::to_string(&YamlValue::Mapping(map.clone())) {
            let _ = util::atomic_write(&temp, serialized.as_bytes(), ENTRY_MODE);
        }
    }
    std::fs::rename(&temp, &pending).map_err(|error| {
        let _ = util::remove_file_if_exists(&temp);
        BoxError::from(error).with_detail("cannot publish downloaded rule profile")
    })?;
    let counts = counts_from_map(&map);
    write_entry(paths, &id, "counts", &counts.json_string())?;
    write_entry(
        paths,
        &id,
        "fetched_at",
        &util::format_unix_utc(util::now_ms() / 1000),
    )?;
    let mut data = item(paths, &id, active_id(paths).as_deref() == Some(id.as_str()))?;
    data["downloaded_bytes"] = json!(report.bytes);
    data["sha256"] = json!(report.sha256);
    data["final_url_masked"] = json!(mask_url(&report.final_url));
    Ok(data)
}

pub fn url(paths: &Paths, params: UrlParams) -> Result<Value> {
    ensure_layout(paths)?;
    let id = params
        .id
        .or_else(|| selected_id(paths).ok().flatten())
        .ok_or_else(|| BoxError::new(ErrorCode::NotFound, "no rule profile is configured"))?;
    let value = read_entry(paths, &id, "url").ok_or_else(|| not_found(&id))?;
    Ok(json!({"id": id, "url": value}))
}

pub fn set(paths: &Paths, params: SetUrlParams) -> Result<Value> {
    ensure_layout(paths)?;
    validate_url(&params.url)?;
    let existing = ids(paths)?;
    if existing.is_empty() {
        let name = params.name.unwrap_or_else(|| "默认规则".to_string());
        validate_name(&name)?;
        write_entry(paths, "default", "name", &name)?;
        write_entry(paths, "default", "url", &params.url)?;
        return get(paths);
    }
    let id = active_id(paths)
        .or_else(|| existing.first().cloned())
        .ok_or_else(|| BoxError::new(ErrorCode::NotFound, "no rule profile is configured"))?;
    let name = read_entry(paths, &id, "name").unwrap_or_else(|| id.clone());
    edit(
        paths,
        EditParams {
            id,
            name,
            url: params.url,
        },
    )?;
    get(paths)
}

pub fn clear(paths: &Paths) -> Result<Value> {
    ensure_layout(paths)?;
    let _ = util::remove_file_if_exists(&paths.rule_profile_active);
    for id in ids(paths)? {
        remove_entry_files(paths, &id)?;
    }
    Ok(json!({"cleared": true}))
}

pub fn apply_with_policy(
    paths: &Paths,
    params: UseParams,
    policy: Option<&Policy>,
) -> Result<Value> {
    use_pending_with_policy(paths, params, policy)
}

#[cfg(test)]
pub fn use_pending(paths: &Paths, params: UseParams) -> Result<Value> {
    use_pending_with_policy(paths, params, None)
}

pub fn use_pending_with_policy(
    paths: &Paths,
    params: UseParams,
    policy: Option<&Policy>,
) -> Result<Value> {
    ensure_layout(paths)?;
    let id = params
        .id
        .or_else(|| selected_id(paths).ok().flatten())
        .ok_or_else(|| BoxError::new(ErrorCode::NotFound, "no rule profile is configured"))?;
    if !entry_exists(paths, &id) {
        return Err(not_found(&id));
    }
    let pending = entry_path(paths, &id, "pending.yaml");
    if !pending.is_file() {
        if let Some(policy) = policy {
            fetch(paths, FetchParams { id: Some(id.clone()) }, policy)?;
        } else {
            return Err(BoxError::new(
                ErrorCode::NotFound,
                "rule profile has no pending YAML; fetch it first",
            ));
        }
    }
    let map = yaml::read_map(&pending)?;
    let groups = section_json(&map, "proxy-groups", json!([]))?;
    let mut providers = section_json(&map, "rule-providers", json!({}))?;
    // Preserve DNS-referenced rule-providers from existing config that the template does not redefine.
    preserve_dns_providers_in_json(paths, &mut providers);
    let rules = section_json(&map, "rules", json!([]))?;
    let data = json!({
        "proxy-groups": groups,
        "rule-providers": providers,
        "rules": rules
    });
    let changed = config::replace_sections(paths, &data)?;
    config::sync_ruleset_prefix_from_settings(paths)?;
    let counts = Counts {
        groups: data["proxy-groups"].as_array().map_or(0, Vec::len),
        providers: data["rule-providers"].as_object().map_or(0, serde_json::Map::len),
        rules: data["rules"].as_array().map_or(0, Vec::len),
    };
    let now = util::format_unix_utc(util::now_ms() / 1000);
    write_entry(paths, &id, "applied_at", &now)?;
    write_entry(paths, &id, "counts", &counts.json_string())?;
    util::atomic_write(
        &paths.rule_profile_active,
        format!("{id}\n").as_bytes(),
        ENTRY_MODE,
    )?;
    let _ = util::remove_file_if_exists(&pending);
    let name = read_entry(paths, &id, "name").unwrap_or_else(|| id.clone());
    // Honor legacy `--reload` semantics: hot-reload ONLY if core is running; when stopped,
    // return `ok: true, reloaded: false` without starting the core!
    let reloaded = if params.reload {
        crate::service::reload_if_running(paths)
            .map_err(|error| error.with_detail("rule profile applied but core reload failed"))?
    } else {
        false
    };
    Ok(json!({
        "id": id,
        "name": name,
        "changed": changed["changed"],
        "reloaded": reloaded,
        "updated_at": now,
        "counts": counts.json()
    }))
}

fn preserve_dns_providers_in_json(paths: &Paths, providers: &mut Value) {
    if let Ok(current) = config::get(paths) {
        if let (Some(target), Some(existing)) = (
            providers.as_object_mut(),
            current.get("rule-providers").and_then(Value::as_object),
        ) {
            let referenced = dns_referenced_providers(paths);
            for name in referenced {
                if let Some(value) = existing.get(&name) {
                    target.entry(name).or_insert_with(|| value.clone());
                }
            }
        }
    }
}

fn dns_referenced_providers(paths: &Paths) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    let Ok(map) = yaml::read_map(&paths.config) else {
        return names;
    };
    let Some(dns) = map
        .get(YamlValue::String("dns".to_string()))
        .and_then(YamlValue::as_mapping)
    else {
        return names;
    };
    if let Some(items) = dns
        .get(YamlValue::String("fake-ip-filter".to_string()))
        .and_then(YamlValue::as_sequence)
    {
        for item in items {
            if let Some(value) = item
                .as_str()
                .and_then(|value| value.strip_prefix("rule-set:"))
            {
                if !value.is_empty() {
                    names.insert(value.to_string());
                }
            }
        }
    }
    if let Some(items) = dns
        .get(YamlValue::String("nameserver-policy".to_string()))
        .and_then(YamlValue::as_mapping)
    {
        for key in items.keys() {
            if let Some(value) = key
                .as_str()
                .and_then(|value| value.strip_prefix("rule-set:"))
            {
                if !value.is_empty() {
                    names.insert(value.to_string());
                }
            }
        }
    }
    names
}

fn item(paths: &Paths, id: &str, active: bool) -> Result<Value> {
    let name = read_entry(paths, id, "name").unwrap_or_else(|| id.to_string());
    let url = read_entry(paths, id, "url").unwrap_or_default();
    let pending = entry_path(paths, id, "pending.yaml");
    let pending_exists = pending.is_file();
    let fetched = if let Some(value) = read_entry(paths, id, "fetched_at") {
        value
    } else if pending_exists {
        file_time(&pending)
    } else {
        String::new()
    };
    let applied = read_entry(paths, id, "applied_at").unwrap_or_default();
    let counts = if pending_exists {
        yaml::read_map(&pending)
            .map(|map| counts_from_map(&map))
            .unwrap_or_else(|_| zero_counts())
    } else if active {
        yaml::read_map(&paths.config)
            .map(|map| counts_from_map(&map))
            .unwrap_or_else(|_| zero_counts())
    } else {
        read_counts(paths, id).unwrap_or_else(zero_counts)
    };
    Ok(json!({
        "id": id,
        "name": name,
        "url_masked": mask_url(&url),
        "active": active,
        "pending": pending_exists,
        "fetched_at": fetched,
        "applied_at": applied,
        "counts": counts.json(),
        "can_delete": !active
    }))
}

fn ensure_layout(paths: &Paths) -> Result<()> {
    util::ensure_dir(&paths.rule_profiles_dir, true)?;
    // Legacy single-file rule-profile migration (`mihomo/rule-profile.url` -> `rule-profiles/default.*`).
    let legacy_url = paths.home.join("mihomo").join("rule-profile.url");
    if legacy_url.is_file() && ids(paths).map(|v| v.is_empty()).unwrap_or(false) {
        if let Ok(url_text) = fs::read_to_string(&legacy_url) {
            let trimmed = url_text.trim();
            if !trimmed.is_empty() {
                let _ = util::atomic_write(
                    &entry_path(paths, "default", "name"),
                    "默认规则\n".as_bytes(),
                    ENTRY_MODE,
                );
                let _ = util::atomic_write(
                    &entry_path(paths, "default", "url"),
                    format!("{trimmed}\n").as_bytes(),
                    ENTRY_MODE,
                );
                let legacy_pending = paths
                    .home
                    .join("run")
                    .join("state")
                    .join("rule-profile.pending.yaml");
                if legacy_pending.is_file() {
                    let _ = fs::rename(&legacy_pending, entry_path(paths, "default", "pending.yaml"));
                }
                let legacy_updated = paths
                    .home
                    .join("run")
                    .join("state")
                    .join("rule-profile.updated_at");
                if legacy_updated.is_file() {
                    let _ = fs::rename(&legacy_updated, entry_path(paths, "default", "applied_at"));
                }
                let _ = util::atomic_write(
                    &paths.rule_profile_active,
                    b"default\n",
                    ENTRY_MODE,
                );
                let _ = util::remove_file_if_exists(&legacy_url);
            }
        }
    }
    Ok(())
}

fn ids(paths: &Paths) -> Result<Vec<String>> {
    if !paths.rule_profiles_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(&paths.rule_profiles_dir)
        .map_err(|error| BoxError::from(error).with_detail("cannot read rule profile directory"))?
    {
        let entry = entry.map_err(BoxError::from)?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(id) = name.strip_suffix(".url") else {
            continue;
        };
        if !id.is_empty() && entry.path().is_file() {
            result.push(id.to_string());
        }
    }
    result.sort();
    result.dedup();
    Ok(result)
}

fn selected_id(paths: &Paths) -> Result<Option<String>> {
    if let Some(active) = active_id(paths) {
        return Ok(Some(active));
    }
    Ok(ids(paths)?.into_iter().next())
}

fn active_id(paths: &Paths) -> Option<String> {
    let text = util::read_to_string_optional(&paths.rule_profile_active)
        .ok()
        .flatten()?;
    let id = text.trim();
    if id.is_empty() || !entry_exists(paths, id) {
        return None;
    }
    Some(id.to_string())
}

fn entry_exists(paths: &Paths, id: &str) -> bool {
    entry_path(paths, id, "url").is_file()
}

fn entry_path(paths: &Paths, id: &str, suffix: &str) -> PathBuf {
    paths.rule_profiles_dir.join(format!("{id}.{suffix}"))
}

fn read_entry(paths: &Paths, id: &str, suffix: &str) -> Option<String> {
    util::read_to_string_optional(&entry_path(paths, id, suffix))
        .ok()
        .flatten()
        .map(|value| value.trim_end_matches('\n').to_string())
}

fn write_entry(paths: &Paths, id: &str, suffix: &str, value: &str) -> Result<()> {
    ensure_layout(paths)?;
    let mut text = value.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    util::atomic_write(&entry_path(paths, id, suffix), text.as_bytes(), ENTRY_MODE)
}

fn remove_entry_files(paths: &Paths, id: &str) -> Result<()> {
    for suffix in [
        "url",
        "name",
        "pending.yaml",
        "pending.yaml.bak",
        "fetched_at",
        "applied_at",
        "counts",
    ] {
        util::remove_file_if_exists(&entry_path(paths, id, suffix))?;
    }
    Ok(())
}

fn unique_id(paths: &Paths, name: &str) -> Result<String> {
    let base = safe_id(name);
    let mut candidate = base.clone();
    let mut index = 2;
    while entry_exists(paths, &candidate) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    Ok(candidate)
}

fn safe_id(name: &str) -> String {
    let mut value = name
        .chars()
        .map(|character| {
            if character.is_whitespace() || matches!(character, '/' | '\\' | '|') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    value = value.trim_start_matches('.').to_string();
    if value.is_empty() {
        "rule".to_string()
    } else {
        value
    }
}

fn section_json(map: &Mapping, key: &str, default: Value) -> Result<Value> {
    match map.get(YamlValue::String(key.to_string())) {
        Some(value) => yaml::to_json(value),
        None => Ok(default),
    }
}

fn zero_counts() -> Counts {
    Counts {
        groups: 0,
        providers: 0,
        rules: 0,
    }
}

fn counts_from_map(map: &Mapping) -> Counts {
    let groups = map
        .get(YamlValue::String("proxy-groups".to_string()))
        .and_then(YamlValue::as_sequence)
        .map_or(0, Vec::len);
    let providers = map
        .get(YamlValue::String("rule-providers".to_string()))
        .and_then(YamlValue::as_mapping)
        .map_or(0, |value| value.len());
    let rules = map
        .get(YamlValue::String("rules".to_string()))
        .and_then(YamlValue::as_sequence)
        .map_or(0, Vec::len);
    Counts {
        groups,
        providers,
        rules,
    }
}

fn read_counts(paths: &Paths, id: &str) -> Option<Counts> {
    let text = read_entry(paths, id, "counts")?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some(Counts {
        groups: value.get("groups").and_then(Value::as_u64).unwrap_or(0) as usize,
        providers: value
            .get("rule_providers")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        rules: value.get("rules").and_then(Value::as_u64).unwrap_or(0) as usize,
    })
}

#[derive(Debug, Clone, Copy)]
struct Counts {
    groups: usize,
    providers: usize,
    rules: usize,
}

impl Counts {
    fn json(self) -> Value {
        json!({
            "groups": self.groups,
            "rule_providers": self.providers,
            "rules": self.rules
        })
    }

    fn json_string(self) -> String {
        serde_json::to_string(&self.json()).unwrap_or_else(|_| "{}".to_string())
    }
}

fn file_time(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(String::new, |duration| {
            util::format_unix_utc(duration.as_secs())
        })
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > 255 {
        return Err(BoxError::validation("rule profile name is invalid"));
    }
    if name.chars().any(char::is_control) {
        return Err(BoxError::validation(
            "rule profile name contains a control character",
        ));
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<()> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(BoxError::validation(
            "rule profile URL must start with http:// or https://",
        ));
    }
    if url.chars().any(char::is_control) {
        return Err(BoxError::validation(
            "rule profile URL contains a control character",
        ));
    }
    Ok(())
}

fn mask_url(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}/***")
        }
        None => "/***".to_string(),
    }
}

fn not_found(id: &str) -> BoxError {
    BoxError::new(
        ErrorCode::NotFound,
        format!("rule profile does not exist: {id}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        add, clear, deactivate, edit, get, list, remove, set, url, use_pending, AddParams,
        EditParams, IdParams, SetUrlParams, UrlParams, UseParams,
    };
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
    }

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "boxctl-rule-profile-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("mihomo")).expect("mkdir");
        fs::copy(
            repo_root().join("box/mihomo/config.yaml"),
            root.join("mihomo/config.yaml"),
        )
        .expect("copy config");
        root
    }

    fn add_one(paths: &Paths) -> String {
        add(
            paths,
            AddParams {
                name: "Main rules".to_string(),
                url: "https://example.com/rules.yaml".to_string(),
            },
        )
        .expect("add")["id"]
            .as_str()
            .expect("id")
            .to_string()
    }

    #[test]
    fn add_list_get_edit_deactivate_remove() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let id = add_one(&paths);
        let listed = list(&paths).expect("list");
        assert_eq!(listed["items"].as_array().expect("items").len(), 1);
        assert_eq!(listed["items"][0]["id"], id);
        assert_eq!(listed["items"][0]["url_masked"], "https://example.com/***");
        let detail = get(&paths).expect("get");
        assert_eq!(detail["configured"], true);
        assert_eq!(detail["name"], "Main rules");

        let edited = edit(
            &paths,
            EditParams {
                id: id.clone(),
                name: "Renamed rules".to_string(),
                url: "https://example.com/other.yaml".to_string(),
            },
        )
        .expect("edit");
        assert_eq!(edited["name"], "Renamed rules");
        deactivate(&paths).expect("deactivate");
        let removed = remove(&paths, IdParams { id }).expect("remove");
        assert_eq!(removed["removed"], true);
        assert!(list(&paths).expect("list")["items"]
            .as_array()
            .expect("items")
            .is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn use_pending_applies_config_and_marks_active() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let id = add_one(&paths);
        let pending = paths.rule_profiles_dir.join(format!("{id}.pending.yaml"));
        fs::write(
            &pending,
            r#"proxy-groups:
  - name: TEST
    type: select
    proxies:
      - DIRECT
rule-providers:
  custom:
    type: http
    behavior: domain
    url: https://example.com/custom.yaml
    path: ./rules/custom.yaml
rules:
  - RULE-SET,custom,TEST
  - MATCH,DIRECT
"#,
        )
        .expect("pending");
        let result = use_pending(
            &paths,
            UseParams {
                id: Some(id.clone()),
                reload: false,
            },
        )
        .expect("use");
        assert_eq!(result["id"], id);
        assert_eq!(result["reloaded"], false);
        assert_eq!(result["counts"]["groups"], 1);
        assert!(!pending.exists());
        assert!(paths.rule_profile_active.exists());
        let detail = get(&paths).expect("get");
        assert_eq!(detail["configured"], true);
        assert_eq!(detail["counts"]["rules"], 2);
        let config = crate::config::get(&paths).expect("config");
        assert!(config["rule-providers"].get("custom").is_some());
        assert!(config["rule-providers"].get("cn_domain").is_some());
        assert!(config["rule-providers"].get("Fake-IP-Filter").is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn url_set_and_clear_round_trip() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        add(
            &paths,
            AddParams {
                name: "template".to_string(),
                url: "https://example.com/one.yaml".to_string(),
            },
        )
        .expect("add");
        let updated = set(
            &paths,
            SetUrlParams {
                url: "https://example.com/two.yaml".to_string(),
                name: None,
            },
        )
        .expect("set");
        assert_eq!(updated["configured"], true);
        let current = url(&paths, UrlParams { id: None }).expect("url");
        assert_eq!(current["url"], "https://example.com/two.yaml");
        clear(&paths).expect("clear");
        let list = list(&paths).expect("list");
        assert!(list["items"].as_array().expect("items").is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_profile_cannot_be_removed() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let id = add_one(&paths);
        let pending = paths.rule_profiles_dir.join(format!("{id}.pending.yaml"));
        fs::write(
            &pending,
            r#"proxy-groups: []
rule-providers: {}
rules: []
"#,
        )
        .expect("pending");
        use_pending(
            &paths,
            UseParams {
                id: Some(id.clone()),
                reload: false,
            },
        )
        .expect("use");
        let error = remove(&paths, IdParams { id }).expect_err("active remove");
        assert_eq!(error.code, crate::error::ErrorCode::Validation);
        let _ = fs::remove_dir_all(root);
    }
}
