use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::{json, Map, Value};
use serde_yaml::Value as YamlValue;

use crate::error::{BoxError, Result};
use crate::paths::Paths;
use crate::yaml;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyParams {
    #[serde(rename = "proxy-groups")]
    pub proxy_groups: Vec<Value>,
    #[serde(rename = "rule-providers")]
    pub rule_providers: Map<String, Value>,
    pub rules: Vec<String>,
}

pub fn get(paths: &Paths) -> Result<Value> {
    let map = yaml::read_map(&paths.config)?;
    let groups = map
        .get(YamlValue::String("proxy-groups".to_string()))
        .map_or_else(|| Ok(json!([])), yaml::to_json)?;
    let providers = map
        .get(YamlValue::String("rule-providers".to_string()))
        .map_or_else(|| Ok(json!({})), yaml::to_json)?;
    let rules = map
        .get(YamlValue::String("rules".to_string()))
        .map_or_else(|| Ok(json!([])), yaml::to_json)?;
    if !groups.is_array() {
        return Err(BoxError::validation("proxy-groups must be an array"));
    }
    if !providers.is_object() {
        return Err(BoxError::validation("rule-providers must be an object"));
    }
    if !rules.is_array() {
        return Err(BoxError::validation("rules must be an array"));
    }
    Ok(json!({
        "proxy-groups": groups,
        "rule-providers": providers,
        "rules": rules
    }))
}

pub fn apply(paths: &Paths, params: ApplyParams) -> Result<Value> {
    validate(&params)?;
    let mut map = yaml::read_map(&paths.config)?;
    map.insert(
        YamlValue::String("proxy-groups".to_string()),
        yaml::from_json(&Value::Array(params.proxy_groups.clone()))?,
    );
    map.insert(
        YamlValue::String("rule-providers".to_string()),
        yaml::from_json(&Value::Object(params.rule_providers.clone()))?,
    );
    map.insert(
        YamlValue::String("rules".to_string()),
        yaml::from_json(&Value::Array(
            params
                .rules
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>(),
        ))?,
    );
    yaml::write_map(&paths.config, &map)?;
    Ok(json!({
        "changed": ["proxy-groups", "rule-providers", "rules"]
    }))
}

pub fn validate_sections(data: &Value) -> Result<()> {
    let params: ApplyParams = serde_json::from_value(data.clone()).map_err(|error| {
        BoxError::validation("rule profile must contain proxy-groups, rule-providers, rules")
            .with_detail(error.to_string())
    })?;
    validate(&params)
}

pub fn replace_sections(paths: &Paths, data: &Value) -> Result<Value> {
    let params: ApplyParams = serde_json::from_value(data.clone()).map_err(|error| {
        BoxError::validation("rule profile must contain proxy-groups, rule-providers, rules")
            .with_detail(error.to_string())
    })?;
    apply(paths, params)
}

pub fn sync_ruleset_prefix(paths: &Paths, prefix: &str) -> Result<()> {
    let old = crate::util::read_to_string_optional(&paths.ruleset_prefix_state)?
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/')
        .to_string();
    let new = prefix.trim().trim_end_matches('/').to_string();
    if old.is_empty() && new.is_empty() {
        return Ok(());
    }
    if !paths.config.is_file() {
        return Ok(());
    }
    let mut map = yaml::read_map(&paths.config)?;
    let Some(providers) = map
        .get_mut(YamlValue::String("rule-providers".to_string()))
        .and_then(YamlValue::as_mapping_mut)
    else {
        crate::util::atomic_write(
            &paths.ruleset_prefix_state,
            format!("{new}\n").as_bytes(),
            0o600,
        )?;
        return Ok(());
    };
    for provider in providers.values_mut() {
        let Some(provider) = provider.as_mapping_mut() else {
            continue;
        };
        let Some(url) = provider
            .get(YamlValue::String("url".to_string()))
            .and_then(YamlValue::as_str)
        else {
            continue;
        };
        let mut url = url.to_string();
        if !old.is_empty() {
            let prefix_with_slash = format!("{old}/");
            if let Some(rest) = url.strip_prefix(&prefix_with_slash) {
                url = rest.to_string();
            }
        }
        if !new.is_empty() {
            let prefix_with_slash = format!("{new}/");
            if !url.starts_with(&prefix_with_slash) {
                url = format!("{new}/{url}");
            }
        }
        provider.insert(YamlValue::String("url".to_string()), YamlValue::String(url));
    }
    yaml::write_map(&paths.config, &map)?;
    crate::util::atomic_write(
        &paths.ruleset_prefix_state,
        format!("{new}\n").as_bytes(),
        0o600,
    )?;
    Ok(())
}

pub fn sync_ruleset_prefix_from_settings(paths: &Paths) -> Result<()> {
    let prefix = crate::settings::SettingsStore::open(paths.clone())
        .ok()
        .and_then(|store| store.load().ok())
        .and_then(|settings| settings.get("ruleset_prefix").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    sync_ruleset_prefix(paths, &prefix)
}

pub fn delete(paths: &Paths) -> Result<Value> {
    if !paths.config.exists() {
        return Err(BoxError::new(
            crate::error::ErrorCode::NotFound,
            "mihomo config does not exist",
        )
        .with_detail(paths.config.display().to_string()));
    }
    let target = paths
        .config
        .with_file_name(format!("config.yaml.deleted.{}", crate::util::now_ms()));
    std::fs::rename(&paths.config, &target).map_err(|error| {
        BoxError::from(error).with_detail(format!(
            "cannot move {} to {}",
            paths.config.display(),
            target.display()
        ))
    })?;
    Ok(json!({"path": target.display().to_string()}))
}

fn validate(params: &ApplyParams) -> Result<()> {
    let mut group_names = BTreeSet::new();
    for group in &params.proxy_groups {
        let object = group
            .as_object()
            .ok_or_else(|| BoxError::validation("each proxy-group must be an object"))?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| BoxError::validation("each proxy-group needs a non-empty name"))?;
        if name.chars().any(char::is_control) {
            return Err(BoxError::validation(
                "proxy-group name contains a control character",
            ));
        }
        if !group_names.insert(name.to_string()) {
            return Err(BoxError::validation(format!(
                "duplicate proxy-group name: {name}"
            )));
        }
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| BoxError::validation(format!("proxy-group {name} needs a type")))?;
        if !matches!(
            kind,
            "select" | "url-test" | "fallback" | "load-balance" | "relay"
        ) {
            return Err(BoxError::validation(format!(
                "unsupported proxy-group type: {kind}"
            )));
        }
    }

    for (name, provider) in &params.rule_providers {
        let object = provider.as_object().ok_or_else(|| {
            BoxError::validation(format!("rule-provider {name} must be an object"))
        })?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| BoxError::validation(format!("rule-provider {name} needs a type")))?;
        if !matches!(kind, "http" | "file" | "inline") {
            return Err(BoxError::validation(format!(
                "unsupported rule-provider type for {name}: {kind}"
            )));
        }
        let behavior = object
            .get("behavior")
            .and_then(Value::as_str)
            .ok_or_else(|| BoxError::validation(format!("rule-provider {name} needs behavior")))?;
        if !matches!(behavior, "domain" | "ipcidr" | "classical") {
            return Err(BoxError::validation(format!(
                "unsupported rule-provider behavior for {name}: {behavior}"
            )));
        }
        if let Some(format) = object.get("format") {
            let Some(format) = format.as_str() else {
                return Err(BoxError::validation(format!(
                    "rule-provider {name} format must be a string"
                )));
            };
            if !matches!(format, "" | "yaml" | "mrs" | "text") {
                return Err(BoxError::validation(format!(
                    "unsupported rule-provider format for {name}: {format}"
                )));
            }
        }
        if let Some(interval) = object.get("interval") {
            if interval.as_u64().is_none() {
                return Err(BoxError::validation(format!(
                    "rule-provider {name} interval must be a non-negative integer"
                )));
            }
        }
        if kind == "http" {
            let url = object
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| BoxError::validation(format!("rule-provider {name} needs url")))?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(BoxError::validation(format!(
                    "rule-provider {name} url must use http or https"
                )));
            }
        }
        if kind != "inline" {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| BoxError::validation(format!("rule-provider {name} needs path")))?;
            if path.chars().any(char::is_control) {
                return Err(BoxError::validation(format!(
                    "rule-provider {name} path contains a control character"
                )));
            }
        }
    }

    for rule in &params.rules {
        if rule.chars().any(char::is_control) {
            return Err(BoxError::validation("rule contains a control character"));
        }
        if let Some(rest) = rule.strip_prefix("RULE-SET,") {
            let mut parts = rest.splitn(2, ',');
            let provider = parts.next().unwrap_or_default();
            // Clash rule suffixes such as ",no-resolve" are modifiers, not
            // part of the proxy-group name.
            let target = parts
                .next()
                .unwrap_or_default()
                .split(',')
                .next()
                .unwrap_or_default()
                .trim();
            if !params.rule_providers.contains_key(provider) {
                return Err(BoxError::validation(format!(
                    "rule references a missing rule-provider: {provider}"
                )));
            }
            if !group_names.contains(target)
                && !matches!(
                    target,
                    "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE"
                )
            {
                return Err(BoxError::validation(format!(
                    "rule references a missing proxy-group: {target}"
                )));
            }
        } else if let Some(rest) = rule.strip_prefix("MATCH,") {
            let target = rest.split(',').next().unwrap_or_default().trim();
            if !group_names.contains(target)
                && !matches!(
                    target,
                    "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE"
                )
            {
                return Err(BoxError::validation(format!(
                    "MATCH rule references a missing proxy-group: {target}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply, delete, get, sync_ruleset_prefix, ApplyParams};
    use crate::paths::Paths;
    use serde_json::{json, Map};
    use std::fs;
    use std::path::{Path, PathBuf};
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
        let root =
            std::env::temp_dir().join(format!("boxctl-config-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("mihomo")).expect("mkdir");
        fs::copy(
            repo_root().join("box/mihomo/config.yaml"),
            root.join("mihomo/config.yaml"),
        )
        .expect("copy config");
        root
    }

    fn params(rules: Vec<&str>) -> ApplyParams {
        let mut providers = Map::new();
        providers.insert(
            "custom".to_string(),
            json!({
                "type": "http",
                "behavior": "domain",
                "url": "https://example.com/custom.yaml",
                "path": "./rules/custom.yaml"
            }),
        );
        ApplyParams {
            proxy_groups: vec![json!({
                "name": "TEST",
                "type": "select",
                "proxies": ["DIRECT"]
            })],
            rule_providers: providers,
            rules: rules.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn get_returns_the_three_sections() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let data = get(&paths).expect("get");
        assert!(data["proxy-groups"].is_array());
        assert!(data["rule-providers"].is_object());
        assert!(data["rules"].is_array());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn valid_apply_replaces_sections() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let result =
            apply(&paths, params(vec!["RULE-SET,custom,TEST", "MATCH,DIRECT"])).expect("apply");
        assert_eq!(result["changed"].as_array().expect("changed").len(), 3);
        let after = get(&paths).expect("get after");
        assert_eq!(after["proxy-groups"][0]["name"], "TEST");
        assert_eq!(after["rule-providers"]["custom"]["type"], "http");
        assert_eq!(after["rules"][0], "RULE-SET,custom,TEST");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rule_set_target_allows_no_resolve_suffix() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        apply(
            &paths,
            params(vec!["RULE-SET,custom,TEST,no-resolve", "MATCH,DIRECT"]),
        )
        .expect("rule modifiers must not be treated as a group name");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_reference_does_not_write() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let before = fs::read(&paths.config).expect("before");
        let error =
            apply(&paths, params(vec!["RULE-SET,missing,TEST"])).expect_err("missing provider");
        assert_eq!(error.code, crate::error::ErrorCode::Validation);
        assert_eq!(fs::read(&paths.config).expect("after"), before);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ruleset_prefix_rewrites_once_and_restores() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let mut map = crate::yaml::read_map(&paths.config).expect("map");
        let mut provider = serde_yaml::Mapping::new();
        provider.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String("http".to_string()),
        );
        provider.insert(
            serde_yaml::Value::String("url".to_string()),
            serde_yaml::Value::String("https://raw.githubusercontent.com/a/r.yaml".to_string()),
        );
        let mut providers = serde_yaml::Mapping::new();
        providers.insert(
            serde_yaml::Value::String("r".to_string()),
            serde_yaml::Value::Mapping(provider),
        );
        map.insert(
            serde_yaml::Value::String("rule-providers".to_string()),
            serde_yaml::Value::Mapping(providers),
        );
        crate::yaml::write_map(&paths.config, &map).expect("write");
        sync_ruleset_prefix(&paths, "https://ghfast.top").expect("prefix");
        let first = fs::read_to_string(&paths.config).expect("first");
        assert!(first.contains("https://ghfast.top/https://raw.githubusercontent.com/a/r.yaml"));
        sync_ruleset_prefix(&paths, "https://ghfast.top").expect("prefix twice");
        let second = fs::read_to_string(&paths.config).expect("second");
        assert_eq!(first, second);
        assert!(!second.contains("ghfast.top/https://ghfast.top"));
        sync_ruleset_prefix(&paths, "").expect("clear");
        let cleared = fs::read_to_string(&paths.config).expect("cleared");
        assert!(cleared.contains("https://raw.githubusercontent.com/a/r.yaml"));
        assert!(!cleared.contains("ghfast.top/https://raw.githubusercontent.com/a/r.yaml"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_moves_config() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let result = delete(&paths).expect("delete");
        let moved = Path::new(result["path"].as_str().expect("path"));
        assert!(!paths.config.exists());
        assert!(moved.exists());
        let _ = fs::remove_dir_all(root);
    }
}
