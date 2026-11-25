use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::Engine as _;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::download::{self, Policy};
use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;
use serde_yaml::{Mapping, Value as YamlValue};

const LIST_MODE: u32 = 0o600;
const UA_MODE: u32 = 0o600;
const FALLBACK_UA: &str = "clash-verge/v2.4.5";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    url: String,
    enabled: bool,
}

#[derive(Debug, Clone)]
struct ProviderSpec {
    name: String,
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameParams {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddParams {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateParams {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    pub old_name: String,
    #[serde(default)]
    pub new_name: Option<String>,
    pub url: String,
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
struct Store {
    lines: Vec<String>,
}

impl Store {
    fn load(path: &Path) -> Result<Self> {
        let raw = util::read_to_string_optional(path)?.unwrap_or_default();
        Ok(Self {
            lines: raw.lines().map(str::to_string).collect(),
        })
    }

    fn entries(&self) -> Vec<Entry> {
        self.lines
            .iter()
            .filter_map(|line| parse_entry(line))
            .collect()
    }

    fn find(&self, name: &str) -> Option<Entry> {
        self.entries().into_iter().find(|entry| entry.name == name)
    }

    fn save(&self, path: &Path) -> Result<()> {
        let mut text = self.lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        util::atomic_write(path, text.as_bytes(), LIST_MODE)
    }
}

fn parse_entry(line: &str) -> Option<Entry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split('|');
    let name = parts.next()?.to_string();
    let url = parts.next()?.to_string();
    if name.is_empty() || url.is_empty() {
        return None;
    }
    let enabled = parts.next() != Some("0");
    Some(Entry { name, url, enabled })
}

impl Entry {
    fn line(&self) -> String {
        if self.enabled {
            format!("{}|{}", self.name, self.url)
        } else {
            format!("{}|{}|0", self.name, self.url)
        }
    }
}

pub fn list(paths: &Paths, global_ua: &str) -> Result<Value> {
    let store = Store::load(&paths.subscriptions)?;
    let items = store
        .entries()
        .into_iter()
        .map(|entry| item(paths, &entry, global_ua, false))
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"items": items}))
}

pub fn count(paths: &Paths) -> Result<usize> {
    Ok(Store::load(&paths.subscriptions)?.entries().len())
}

pub fn get(paths: &Paths, params: NameParams, global_ua: &str) -> Result<Value> {
    let store = Store::load(&paths.subscriptions)?;
    let entry = store
        .find(&params.name)
        .ok_or_else(|| not_found(&params.name))?;
    item(paths, &entry, global_ua, true)
}

pub fn add(paths: &Paths, params: AddParams, global_ua: &str) -> Result<Value> {
    let url = params.url.trim().to_string();
    validate_url(&url)?;
    let name = params
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| host_from_url(&url));
    validate_name(&name)?;
    if let Some(ua) = &params.user_agent {
        validate_ua(ua)?;
    }
    let mut store = Store::load(&paths.subscriptions)?;
    if store.find(&name).is_some() {
        return Err(BoxError::new(
            ErrorCode::Conflict,
            format!("subscription already exists: {name}"),
        ));
    }
    ensure_safe_name_unique(&store.entries(), &name, None)?;
    let old = store.clone();
    store.lines.push(
        Entry {
            name: name.clone(),
            url: url.clone(),
            enabled: true,
        }
        .line(),
    );
    store.save(&paths.subscriptions)?;
    if let Some(ua) = &params.user_agent {
        if let Err(error) = set_override_ua(paths, &name, ua) {
            let _ = old.save(&paths.subscriptions);
            return Err(error);
        }
    }
    let entry = store.find(&name).expect("entry was added");
    item(paths, &entry, global_ua, true)
}

pub fn set(paths: &Paths, params: SetParams, global_ua: &str) -> Result<Value> {
    let mut store = Store::load(&paths.subscriptions)?;
    let old_entry = store
        .find(&params.old_name)
        .ok_or_else(|| not_found(&params.old_name))?;
    let new_name = params
        .new_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| params.old_name.clone());
    validate_name(&new_name)?;
    validate_url(&params.url)?;
    if let Some(ua) = &params.user_agent {
        validate_ua(ua)?;
    }
    if new_name != params.old_name && store.find(&new_name).is_some() {
        return Err(BoxError::new(
            ErrorCode::Conflict,
            format!("subscription already exists: {new_name}"),
        ));
    }
    ensure_safe_name_unique(&store.entries(), &new_name, Some(&params.old_name))?;
    let old_safe = safe_name(&params.old_name);
    let new_safe = safe_name(&new_name);
    let old_provider = provider_file(paths, &old_safe);
    let new_provider = provider_file(paths, &new_safe);
    if old_safe != new_safe && new_provider.exists() {
        return Err(BoxError::new(
            ErrorCode::Conflict,
            "subscription name generates a conflicting provider file",
        ));
    }

    let old_lines = store.lines.clone();
    let mut replaced = false;
    for line in &mut store.lines {
        if let Some(mut entry) = parse_entry(line) {
            if entry.name == params.old_name {
                entry.name = new_name.clone();
                entry.url = params.url.clone();
                *line = entry.line();
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        return Err(BoxError::internal("subscription entry disappeared"));
    }
    store.save(&paths.subscriptions)?;

    if old_safe != new_safe && old_provider.exists() {
        if let Err(error) = fs::rename(&old_provider, &new_provider) {
            let _ = store_with_lines(&old_lines).save(&paths.subscriptions);
            return Err(BoxError::from(error).with_detail("cannot rename provider file"));
        }
    }

    let ua_result = match &params.user_agent {
        None if old_safe != new_safe => rename_override_ua(paths, &old_safe, &new_safe),
        None => Ok(()),
        Some(value) => {
            if old_safe != new_safe {
                let _ = remove_override_ua(paths, &old_safe);
            }
            set_override_ua(paths, &new_name, value)
        }
    };
    if let Err(error) = ua_result {
        let _ = store_with_lines(&old_lines).save(&paths.subscriptions);
        if old_safe != new_safe && new_provider.exists() {
            let _ = fs::rename(&new_provider, &old_provider);
        }
        return Err(error);
    }

    let entry = Entry {
        name: new_name.clone(),
        url: params.url,
        enabled: old_entry.enabled,
    };
    let mut data = item(paths, &entry, global_ua, true)?;
    data["renamed"] = Value::Bool(new_name != params.old_name);
    Ok(data)
}

pub fn remove(paths: &Paths, params: NameParams) -> Result<Value> {
    let mut store = Store::load(&paths.subscriptions)?;
    if store.find(&params.name).is_none() {
        return Err(not_found(&params.name));
    }
    store
        .lines
        .retain(|line| parse_entry(line).is_none_or(|entry| entry.name != params.name));
    store.save(&paths.subscriptions)?;
    let safe = safe_name(&params.name);
    remove_manifest_source(paths, &params.name)?;
    let _ = util::remove_file_if_exists(&provider_file(paths, &safe));
    let _ =
        util::remove_file_if_exists(&provider_file(paths, &safe).with_extension("yaml.download"));
    let _ = remove_override_ua(paths, &safe);
    Ok(json!({"name": params.name}))
}

pub fn update(
    paths: &Paths,
    params: UpdateParams,
    policy: &Policy,
    keep_last_good: bool,
) -> Result<Value> {
    let store = Store::load(&paths.subscriptions)?;
    if let Some(name) = &params.name {
        if store.find(name).is_none() {
            return Err(not_found(name));
        }
    }
    util::ensure_dir(&paths.providers_dir, true)?;
    let mut success = 0usize;
    let mut failures = 0usize;
    let mut content_failures = 0usize;
    let mut committed_providers = Vec::new();
    for entry in store.entries() {
        if let Some(name) = &params.name {
            if entry.name != *name {
                continue;
            }
        } else if !entry.enabled {
            continue;
        }
        let mut entry_policy = policy.clone();
        entry_policy.user_agent = effective_ua(paths, &entry.name, &policy.user_agent);
        let provider = provider_file(paths, &safe_name(&entry.name));
        let temp = provider.with_extension("yaml.download");
        let _ = util::remove_file_if_exists(&temp);
        let downloaded = download::download_to_file(&temp, &entry.url, &entry_policy, None);
        if let Err(_error) = downloaded {
            let _ = util::remove_file_if_exists(&temp);
            failures += 1;
            if !keep_last_good {
                let _ = util::remove_file_if_exists(&provider);
            }
            continue;
        }
        let bytes = match fs::read(&temp) {
            Ok(bytes) => bytes,
            Err(_) => {
                let _ = util::remove_file_if_exists(&temp);
                failures += 1;
                content_failures += 1;
                if !keep_last_good {
                    let _ = util::remove_file_if_exists(&provider);
                }
                continue;
            }
        };
        match parse_aggregate(&bytes) {
            Ok(Some(specs)) => {
                let aggregate =
                    update_aggregate(paths, &entry.name, &specs, &entry_policy, keep_last_good);
                let _ = util::remove_file_if_exists(&temp);
                match aggregate {
                    Ok((true, aggregate_failures)) => {
                        success += 1;
                        failures += aggregate_failures;
                    }
                    Ok((false, _)) => {
                        failures += 1;
                        content_failures += 1;
                    }
                    Err(error) => {
                        return Err(error);
                    }
                }
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                let _ = util::remove_file_if_exists(&temp);
                failures += 1;
                content_failures += 1;
                continue;
            }
        }
        let parsed = match parse_provider(&bytes) {
            Ok(parsed) => parsed,
            Err(_) => {
                let _ = util::remove_file_if_exists(&temp);
                failures += 1;
                content_failures += 1;
                if !keep_last_good {
                    let _ = util::remove_file_if_exists(&provider);
                }
                continue;
            }
        };
        let _ = util::remove_file_if_exists(&temp);
        if let Err(error) = util::atomic_write(&provider, &parsed, LIST_MODE) {
            return Err(error.with_detail("cannot commit subscription provider"));
        }
        success += 1;
        committed_providers.push((entry.name.clone(), entry.url.clone(), provider));
    }
    if success > 0 {
        sync_proxy_providers(paths, &committed_providers)?;
        restart_core_if_running(paths)?;
    }
    if success == 0 && failures > 0 {
        let code = if content_failures > 0 {
            ErrorCode::Content
        } else {
            ErrorCode::Download
        };
        return Err(BoxError::new(code, "all selected subscriptions failed"));
    }
    Ok(json!({
        "updated": true,
        "partial": failures > 0,
        "failed": failures > 0,
        "success": success,
        "failures": failures
    }))
}

fn parse_aggregate(bytes: &[u8]) -> Result<Option<Vec<ProviderSpec>>> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    let Ok(YamlValue::Mapping(map)) = serde_yaml::from_str::<YamlValue>(text) else {
        return Ok(None);
    };
    let Some(providers) = map.get(YamlValue::String("proxy-providers".to_string())) else {
        return Ok(None);
    };
    if let Some(proxies) = map.get(YamlValue::String("proxies".to_string())) {
        // Aggregate provider URLs commonly include `proxies: []` as a
        // placeholder. Only a non-empty proxy list is actually mixed content.
        if !proxies.as_sequence().is_some_and(Vec::is_empty) {
            return Err(BoxError::new(
                ErrorCode::Content,
                "subscription mixes proxies and proxy-providers",
            ));
        }
    }
    let Some(providers) = providers.as_mapping() else {
        return Err(BoxError::new(
            ErrorCode::Content,
            "proxy-providers must be a mapping",
        ));
    };
    if providers.is_empty() {
        return Err(BoxError::new(
            ErrorCode::Content,
            "proxy-providers is empty",
        ));
    }
    let mut result = Vec::new();
    for (name, value) in providers {
        let Some(name) = name.as_str() else {
            return Err(BoxError::new(
                ErrorCode::Content,
                "proxy-provider name must be a string",
            ));
        };
        if name.is_empty() || name.contains('|') || name.chars().any(char::is_control) {
            return Err(BoxError::new(
                ErrorCode::Content,
                "proxy-provider name is invalid",
            ));
        }
        let Some(value) = value.as_mapping() else {
            return Err(BoxError::new(
                ErrorCode::Content,
                "proxy-provider definition must be a mapping",
            ));
        };
        let Some(url) = value
            .get(YamlValue::String("url".to_string()))
            .and_then(YamlValue::as_str)
        else {
            return Err(BoxError::new(
                ErrorCode::Content,
                "proxy-provider has no url",
            ));
        };
        validate_url(url)?;
        if !url.starts_with("https://") {
            return Err(BoxError::new(
                ErrorCode::Tls,
                "aggregate proxy-provider URL must use https",
            ));
        }
        result.push(ProviderSpec {
            name: name.to_string(),
            url: url.to_string(),
        });
    }
    Ok(Some(result))
}

fn update_aggregate(
    paths: &Paths,
    source: &str,
    specs: &[ProviderSpec],
    policy: &Policy,
    keep_last_good: bool,
) -> Result<(bool, usize)> {
    util::ensure_dir(&paths.providers_dir, true)?;
    let mut success = Vec::new();
    let mut failures = 0;
    let existing = Store::load(&paths.subscriptions)?;
    for spec in specs {
        let safe = safe_name(&spec.name);
        if existing
            .entries()
            .iter()
            .any(|entry| entry.name != source && safe_name(&entry.name) == safe)
        {
            return Err(BoxError::new(
                ErrorCode::Conflict,
                format!(
                    "aggregate provider conflicts with subscription: {}",
                    spec.name
                ),
            ));
        }
        let provider = provider_file(paths, &safe);
        let temp = provider.with_extension("yaml.download");
        let _ = util::remove_file_if_exists(&temp);
        if download::download_to_file(&temp, &spec.url, policy, None).is_err() {
            let _ = util::remove_file_if_exists(&temp);
            failures += 1;
            if !keep_last_good {
                let _ = util::remove_file_if_exists(&provider);
            }
            continue;
        }
        let bytes = match fs::read(&temp) {
            Ok(bytes) => bytes,
            Err(_) => {
                let _ = util::remove_file_if_exists(&temp);
                failures += 1;
                continue;
            }
        };
        let parsed = match parse_provider(&bytes) {
            Ok(parsed) => parsed,
            Err(_) => {
                let _ = util::remove_file_if_exists(&temp);
                failures += 1;
                if !keep_last_good {
                    let _ = util::remove_file_if_exists(&provider);
                }
                continue;
            }
        };
        let _ = util::remove_file_if_exists(&temp);
        util::atomic_write(&provider, &parsed, LIST_MODE)?;
        success.push((spec.name.clone(), spec.url.clone(), provider));
    }
    if success.is_empty() {
        return Ok((false, failures));
    }
    write_manifest(paths, source, &success)?;
    sync_proxy_providers(paths, &success)?;
    Ok((true, failures))
}

fn restart_core_if_running(paths: &Paths) -> Result<()> {
    let running = crate::service::status(paths)
        .ok()
        .and_then(|status| status["running"].as_bool())
        .unwrap_or(false);
    if running {
        crate::service::restart(paths)?;
    }
    Ok(())
}

fn sync_proxy_providers(paths: &Paths, entries: &[(String, String, PathBuf)]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let auto_modify = crate::settings::SettingsStore::open(paths.clone())
        .ok()
        .and_then(|store| store.load().ok())
        .and_then(|settings| settings.get("auto_modify_config").cloned())
        .and_then(|value| value.as_str().map(|value| value == "true"))
        .unwrap_or(true);
    if !auto_modify {
        return Ok(());
    }
    let text = fs::read_to_string(&paths.config)
        .map_err(|error| BoxError::from(error).with_detail("cannot read mihomo config"))?;
    let mut value: YamlValue = serde_yaml::from_str(&text).map_err(|error| {
        BoxError::validation("mihomo config is not valid YAML").with_detail(error.to_string())
    })?;
    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| BoxError::validation("mihomo config must be a mapping"))?;
    let providers_key = YamlValue::String("proxy-providers".to_string());
    let providers = mapping
        .entry(providers_key)
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let providers = providers
        .as_mapping_mut()
        .ok_or_else(|| BoxError::validation("proxy-providers must be a mapping"))?;
    for (name, url, path) in entries {
        let file = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| BoxError::internal("provider path has no file name"))?;
        let mut definition = Mapping::new();
        definition.insert(
            YamlValue::String("type".to_string()),
            YamlValue::String("http".to_string()),
        );
        definition.insert(
            YamlValue::String("url".to_string()),
            YamlValue::String(url.clone()),
        );
        definition.insert(
            YamlValue::String("path".to_string()),
            YamlValue::String(format!("./proxy_provider/{file}")),
        );
        definition.insert(
            YamlValue::String("interval".to_string()),
            YamlValue::Number(86400.into()),
        );
        providers.insert(
            YamlValue::String(name.clone()),
            YamlValue::Mapping(definition),
        );
    }
    let bytes = serde_yaml::to_string(&value).map_err(|error| {
        BoxError::internal("cannot serialize provider config").with_detail(error.to_string())
    })?;
    util::atomic_write(&paths.config, bytes.as_bytes(), 0o600)
}

fn write_manifest(
    paths: &Paths,
    source: &str,
    success: &[(String, String, PathBuf)],
) -> Result<()> {
    let raw = util::read_to_string_optional(&paths.subscriptions_providers)?.unwrap_or_default();
    let mut lines = raw
        .lines()
        .filter(|line| line.split('|').next().is_none_or(|value| value != source))
        .map(str::to_string)
        .collect::<Vec<_>>();
    for (name, url, path) in success {
        lines.push(format!("{source}|{name}|{url}|{}", path.display()));
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    util::ensure_dir(
        paths
            .subscriptions_providers
            .parent()
            .ok_or_else(|| BoxError::internal("manifest path has no parent"))?,
        true,
    )?;
    util::atomic_write(&paths.subscriptions_providers, text.as_bytes(), 0o600)
}

fn manifest_count(paths: &Paths, source: &str) -> Result<usize> {
    let raw = util::read_to_string_optional(&paths.subscriptions_providers)?.unwrap_or_default();
    Ok(raw
        .lines()
        .filter(|line| line.split('|').next() == Some(source))
        .count())
}

fn remove_manifest_source(paths: &Paths, source: &str) -> Result<()> {
    let raw = util::read_to_string_optional(&paths.subscriptions_providers)?.unwrap_or_default();
    let mut lines = Vec::new();
    let mut removed_paths = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split('|');
        if parts.next() == Some(source) {
            let _name = parts.next();
            let _url = parts.next();
            if let Some(path) = parts.next() {
                removed_paths.push(PathBuf::from(path));
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if removed_paths.is_empty() {
        return Ok(());
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    util::atomic_write(&paths.subscriptions_providers, text.as_bytes(), 0o600)?;
    for path in removed_paths {
        let _ = util::remove_file_if_exists(&path);
    }
    Ok(())
}

fn parse_provider(bytes: &[u8]) -> Result<Vec<u8>> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        if let Ok(YamlValue::Mapping(map)) = serde_yaml::from_str::<YamlValue>(text) {
            if map.contains_key(YamlValue::String("proxy-providers".to_string())) {
                return Err(BoxError::new(
                    ErrorCode::Content,
                    "aggregate proxy-providers subscriptions are not supported yet",
                ));
            }
            if let Some(proxies) = map.get(YamlValue::String("proxies".to_string())) {
                if proxies.as_sequence().is_some_and(|items| !items.is_empty()) {
                    let mut output = Mapping::new();
                    output.insert(YamlValue::String("proxies".to_string()), proxies.clone());
                    let text = serde_yaml::to_string(&YamlValue::Mapping(output))
                        .map_err(|error| BoxError::new(ErrorCode::Content, error.to_string()))?;
                    return Ok(text.into_bytes());
                }
            }
        }
        if text.lines().any(|line| {
            [
                "vless://",
                "vmess://",
                "ss://",
                "hysteria://",
                "hysteria2://",
                "anytls://",
                "trojan://",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        }) {
            return Ok(bytes.to_vec());
        }
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(text.trim()) {
            if let Ok(decoded_text) = std::str::from_utf8(&decoded) {
                if decoded_text.lines().any(|line| {
                    [
                        "vless://",
                        "vmess://",
                        "ss://",
                        "hysteria://",
                        "hysteria2://",
                        "anytls://",
                        "trojan://",
                    ]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
                }) {
                    return Ok(decoded);
                }
            }
        }
    }
    Err(BoxError::new(
        ErrorCode::Content,
        "subscription does not contain a supported proxy provider",
    ))
}

pub fn reset(paths: &Paths) -> Result<Value> {
    util::atomic_write(&paths.subscriptions, b"", LIST_MODE)?;
    let _ = util::remove_file_if_exists(&paths.subscriptions_providers);
    if paths.providers_dir.exists() {
        for entry in fs::read_dir(&paths.providers_dir)
            .map_err(|error| BoxError::from(error).with_detail("cannot read provider directory"))?
        {
            let entry = entry.map_err(BoxError::from)?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name.ends_with(".yaml") || name.ends_with(".yaml.download") {
                fs::remove_file(&path).map_err(|error| {
                    BoxError::from(error).with_detail(format!("cannot remove {}", path.display()))
                })?;
            }
        }
    }
    match fs::remove_dir_all(&paths.subscriptions_ua_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(
                BoxError::from(error).with_detail("cannot remove subscription UA directory")
            );
        }
    }
    Ok(json!({"cleared": true}))
}

pub fn set_enabled(
    paths: &Paths,
    params: NameParams,
    enabled: bool,
    auto_modify: bool,
    global_ua: &str,
) -> Result<Value> {
    if !auto_modify {
        return Err(BoxError::validation(
            "auto_modify_config is disabled; proxy-providers cannot be updated",
        ));
    }
    let mut store = Store::load(&paths.subscriptions)?;
    if store.find(&params.name).is_none() {
        return Err(not_found(&params.name));
    }
    let mut found = false;
    for line in &mut store.lines {
        if let Some(mut value) = parse_entry(line) {
            if value.name == params.name {
                value.enabled = enabled;
                *line = value.line();
                found = true;
                break;
            }
        }
    }
    if !found {
        return Err(BoxError::internal("subscription entry disappeared"));
    }
    store.save(&paths.subscriptions)?;
    let safe = safe_name(&params.name);
    let missing = !provider_file(paths, &safe).exists();
    Ok(json!({
        "name": params.name,
        "enabled": enabled,
        "reloaded": false,
        "missing": missing,
        "user_agent_effective": effective_ua(paths, &params.name, global_ua)
    }))
}

fn item(paths: &Paths, entry: &Entry, global_ua: &str, detail: bool) -> Result<Value> {
    let safe = safe_name(&entry.name);
    let provider = provider_file(paths, &safe);
    let provider_count = manifest_count(paths, &entry.name)?;
    let (state, size, updated) = if provider_count > 0 {
        ("aggregated", 0, String::new())
    } else if provider.is_file() {
        let metadata = fs::metadata(&provider).ok();
        let size = metadata.as_ref().map_or(0, |value| value.len());
        let updated = metadata
            .and_then(|value| value.modified().ok())
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map_or_else(String::new, |value| util::format_unix_utc(value.as_secs()));
        if size > 0 {
            ("downloaded", size, updated)
        } else {
            ("missing", size, updated)
        }
    } else {
        ("missing", 0, String::new())
    };
    let override_ua = read_override_ua(paths, &safe)?;
    let effective = if override_ua.is_empty() {
        if global_ua.is_empty() {
            FALLBACK_UA.to_string()
        } else {
            global_ua.to_string()
        }
    } else {
        override_ua.clone()
    };
    let mut data = json!({
        "name": entry.name,
        "url_masked": mask_url(&entry.url),
        "provider": state,
        "provider_size": size,
        "provider_count": provider_count,
        "updated_at": updated,
        "enabled": entry.enabled,
        "user_agent_effective": effective
    });
    if detail {
        data["url"] = Value::String(entry.url.clone());
        data["user_agent"] = Value::String(override_ua);
    }
    Ok(data)
}

fn global_or_fallback(global: &str) -> &str {
    if global.is_empty() {
        FALLBACK_UA
    } else {
        global
    }
}

fn effective_ua(paths: &Paths, name: &str, global: &str) -> String {
    match read_override_ua(paths, &safe_name(name)) {
        Ok(value) if !value.is_empty() => value,
        _ => global_or_fallback(global).to_string(),
    }
}

fn provider_file(paths: &Paths, safe: &str) -> PathBuf {
    paths.providers_dir.join(format!("{safe}.yaml"))
}

fn ua_file(paths: &Paths, safe: &str) -> PathBuf {
    paths.subscriptions_ua_dir.join(format!("{safe}.ua"))
}

fn read_override_ua(paths: &Paths, safe: &str) -> Result<String> {
    Ok(util::read_to_string_optional(&ua_file(paths, safe))?.unwrap_or_default())
}

fn set_override_ua(paths: &Paths, name: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return remove_override_ua(paths, &safe_name(name));
    }
    util::ensure_dir(&paths.subscriptions_ua_dir, true)?;
    util::atomic_write(&ua_file(paths, &safe_name(name)), value.as_bytes(), UA_MODE)
}

fn remove_override_ua(paths: &Paths, safe: &str) -> Result<()> {
    util::remove_file_if_exists(&ua_file(paths, safe))
}

fn rename_override_ua(paths: &Paths, old_safe: &str, new_safe: &str) -> Result<()> {
    let old = ua_file(paths, old_safe);
    let new = ua_file(paths, new_safe);
    if old == new || !old.exists() {
        return Ok(());
    }
    util::ensure_dir(&paths.subscriptions_ua_dir, true)?;
    fs::rename(&old, &new)
        .map_err(|error| BoxError::from(error).with_detail("cannot rename subscription UA"))
}

fn store_with_lines(lines: &[String]) -> Store {
    Store {
        lines: lines.to_vec(),
    }
}

fn ensure_safe_name_unique(entries: &[Entry], name: &str, exclude: Option<&str>) -> Result<()> {
    let safe = safe_name(name);
    for entry in entries {
        if exclude == Some(entry.name.as_str()) {
            continue;
        }
        if safe_name(&entry.name) == safe {
            return Err(BoxError::new(
                ErrorCode::Conflict,
                format!("subscription name conflicts with provider file name: {name}"),
            ));
        }
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<()> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(BoxError::validation(
            "subscription URL must start with http:// or https://",
        ));
    }
    if url.contains('|') || url.chars().any(char::is_control) {
        return Err(BoxError::validation(
            "subscription URL contains an invalid character",
        ));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(BoxError::validation("subscription name must not be empty"));
    }
    if name.chars().count() > 255 {
        return Err(BoxError::validation("subscription name is too long"));
    }
    if name.contains('|') || name.chars().any(char::is_control) {
        return Err(BoxError::validation(
            "subscription name contains an invalid character",
        ));
    }
    Ok(())
}

fn validate_ua(value: &str) -> Result<()> {
    if value.chars().count() > 255 || value.chars().any(char::is_control) {
        return Err(BoxError::validation("subscription User-Agent is invalid"));
    }
    Ok(())
}

fn host_from_url(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest).to_string()
}

fn safe_name(name: &str) -> String {
    let mut safe = name
        .chars()
        .map(|character| {
            if character.is_whitespace() || matches!(character, '/' | '\\' | '|') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    safe = safe.trim_start_matches('.').to_string();
    if safe.is_empty() {
        "sub".to_string()
    } else {
        safe
    }
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

fn not_found(name: &str) -> BoxError {
    BoxError::new(
        ErrorCode::NotFound,
        format!("subscription does not exist: {name}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        add, get, list, parse_aggregate, parse_provider, remove, set, set_enabled, AddParams,
        NameParams, SetParams,
    };
    use crate::paths::Paths;
    use base64::Engine;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root = std::env::temp_dir().join(format!("boxctl-sub-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("mihomo")).expect("mkdir");
        root
    }

    fn ua() -> &'static str {
        "global-agent/1.0"
    }

    #[test]
    fn add_list_get_and_remove() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let added = add(
            &paths,
            AddParams {
                url: "https://example.com/sub?token=secret".to_string(),
                name: Some("main".to_string()),
                user_agent: Some("local-agent/2.0".to_string()),
            },
            ua(),
        )
        .expect("add");
        assert_eq!(added["name"], "main");
        assert_eq!(added["provider"], "missing");
        assert_eq!(added["user_agent_effective"], "local-agent/2.0");
        assert_eq!(added["url"], "https://example.com/sub?token=secret");
        assert_eq!(added["url_masked"], "https://example.com/***");

        let listed = list(&paths, ua()).expect("list");
        assert_eq!(listed["items"].as_array().expect("items").len(), 1);
        let detail = get(
            &paths,
            NameParams {
                name: "main".to_string(),
            },
            ua(),
        )
        .expect("get");
        assert_eq!(detail["user_agent_effective"], "local-agent/2.0");

        let removed = remove(
            &paths,
            NameParams {
                name: "main".to_string(),
            },
        )
        .expect("remove");
        assert_eq!(removed["name"], "main");
        assert!(list(&paths, ua()).expect("list")["items"]
            .as_array()
            .expect("items")
            .is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn set_renames_and_preserves_enabled_state() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        add(
            &paths,
            AddParams {
                url: "https://one.example/sub".to_string(),
                name: Some("old".to_string()),
                user_agent: None,
            },
            ua(),
        )
        .expect("add");
        set_enabled(
            &paths,
            NameParams {
                name: "old".to_string(),
            },
            false,
            true,
            ua(),
        )
        .expect("disable");
        let changed = set(
            &paths,
            SetParams {
                old_name: "old".to_string(),
                new_name: Some("new".to_string()),
                url: "https://two.example/sub".to_string(),
                user_agent: Some("".to_string()),
            },
            ua(),
        )
        .expect("set");
        assert_eq!(changed["name"], "new");
        assert_eq!(changed["renamed"], true);
        assert_eq!(changed["enabled"], false);
        let detail = get(
            &paths,
            NameParams {
                name: "new".to_string(),
            },
            ua(),
        )
        .expect("get");
        assert_eq!(detail["url"], "https://two.example/sub");
        assert_eq!(detail["enabled"], false);
        assert_eq!(detail["user_agent_effective"], ua());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn provider_parser_accepts_yaml_and_uri_lists() {
        let yaml =
            parse_provider(b"proxies:\n  - name: one\n    type: ss\n    server: example.com\n")
                .expect("yaml");
        assert!(String::from_utf8(yaml).expect("utf8").contains("proxies"));
        let uri = parse_provider(b"ss://abc@example.com:443#node\n").expect("uri");
        assert!(String::from_utf8(uri).expect("utf8").contains("ss://"));
        let encoded =
            base64::engine::general_purpose::STANDARD.encode("ss://abc@example.com:443#node\n");
        let decoded = parse_provider(encoded.as_bytes()).expect("base64");
        assert!(String::from_utf8(decoded).expect("utf8").contains("ss://"));
    }

    #[test]
    fn aggregate_parser_reads_provider_urls_and_rejects_mixed() {
        let specs =
            parse_aggregate(b"proxy-providers:\n  p1:\n    url: https://example.com/p.yaml\n")
                .expect("parse")
                .expect("aggregate");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "p1");
        let placeholder = parse_aggregate(
            b"proxies: []\nproxy-providers:\n  p1:\n    url: https://example.com/p.yaml\n",
        )
        .expect("empty proxies placeholder")
        .expect("aggregate");
        assert_eq!(placeholder.len(), 1);
        let mixed = parse_aggregate(
            b"proxies:\n  - name: p\n    type: direct\nproxy-providers:\n  p1:\n    url: https://example.com/p.yaml\n",
        )
        .expect_err("mixed");
        assert_eq!(mixed.code, crate::error::ErrorCode::Content);
        let plain = parse_aggregate(b"proxies: []\n").expect("plain");
        assert!(plain.is_none());
    }

    #[test]
    fn provider_parser_rejects_aggregate_and_garbage() {
        let aggregate = parse_provider(b"proxy-providers:\n  p:\n    url: https://example.com\n")
            .expect_err("aggregate");
        assert_eq!(aggregate.code, crate::error::ErrorCode::Content);
        let garbage = parse_provider(b"not a provider").expect_err("garbage");
        assert_eq!(garbage.code, crate::error::ErrorCode::Content);
    }

    #[test]
    fn duplicate_and_invalid_url_are_rejected() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        add(
            &paths,
            AddParams {
                url: "https://example.com/sub".to_string(),
                name: Some("main".to_string()),
                user_agent: None,
            },
            ua(),
        )
        .expect("add");
        let duplicate = add(
            &paths,
            AddParams {
                url: "https://example.com/other".to_string(),
                name: Some("main".to_string()),
                user_agent: None,
            },
            ua(),
        )
        .expect_err("duplicate");
        assert_eq!(duplicate.code, crate::error::ErrorCode::Conflict);
        let invalid = add(
            &paths,
            AddParams {
                url: "ftp://example.com/sub".to_string(),
                name: Some("other".to_string()),
                user_agent: None,
            },
            ua(),
        )
        .expect_err("invalid url");
        assert_eq!(invalid.code, crate::error::ErrorCode::Validation);
        let _ = fs::remove_dir_all(root);
    }
}
