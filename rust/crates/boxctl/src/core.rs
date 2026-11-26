use serde::Deserialize;
use serde_json::{json, Value};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::error::{BoxError, Result};
use crate::paths::Paths;
use crate::yaml;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredUpdate {
    pub mode: Option<String>,
    #[serde(rename = "log-level")]
    pub log_level: Option<String>,
    #[serde(rename = "allow-lan")]
    pub allow_lan: Option<bool>,
    #[serde(rename = "external-controller")]
    pub external_controller: Option<String>,
    #[serde(rename = "external-ui")]
    pub external_ui: Option<String>,
    #[serde(rename = "profile.store-selected")]
    pub profile_store_selected: Option<bool>,
    #[serde(rename = "profile.store-fake-ip")]
    pub profile_store_fake_ip: Option<bool>,
    #[serde(rename = "unified-delay")]
    pub unified_delay: Option<bool>,
    #[serde(rename = "tcp-concurrent")]
    pub tcp_concurrent: Option<bool>,
    #[serde(rename = "find-process-mode")]
    pub find_process_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyParams {
    pub raw: Option<String>,
    pub structured: Option<StructuredUpdate>,
}

pub fn get(paths: &Paths) -> Result<Value> {
    let map = yaml::read_map(&paths.config)?;
    let raw = yaml::read_raw(&paths.config)?;
    Ok(json!({
        "core": "mihomo",
        "file": paths.config.display().to_string(),
        "structured": structured_json(&map)?,
        "raw": raw
    }))
}

pub fn apply(paths: &Paths, params: ApplyParams) -> Result<Value> {
    match (params.raw, params.structured) {
        (Some(_), Some(_)) => Err(BoxError::validation(
            "raw and structured cannot be submitted together",
        )),
        (None, None) => Err(BoxError::validation("missing raw or structured")),
        (Some(raw), None) => {
            yaml::write_raw(&paths.config, &raw)?;
            Ok(json!({"changed": ["mihomo"], "mode": "raw"}))
        }
        (None, Some(update)) => {
            let mut map = yaml::read_map(&paths.config)?;
            apply_structured(&mut map, &update)?;
            yaml::write_map(&paths.config, &map)?;
            Ok(json!({"changed": ["mihomo"], "mode": "structured"}))
        }
    }
}

fn structured_json(map: &Mapping) -> Result<Value> {
    Ok(json!({
        "mode": string_value(map, "mode", "rule", &["rule", "global", "direct"])?,
        "log-level": string_value(map, "log-level", "error", &["silent", "error", "warning", "info", "debug"])?,
        "allow-lan": bool_value(map, "allow-lan", false)?,
        "external-controller": string_value(map, "external-controller", "", &[])?,
        "external-ui": string_value(map, "external-ui", "", &[])?,
        "profile.store-selected": profile_bool(map, "store-selected", true)?,
        "profile.store-fake-ip": profile_bool(map, "store-fake-ip", true)?,
        "unified-delay": bool_value(map, "unified-delay", false)?,
        "tcp-concurrent": bool_value(map, "tcp-concurrent", true)?,
        "find-process-mode": string_value(map, "find-process-mode", "off", &["always", "strict", "off"])?
    }))
}

fn apply_structured(map: &mut Mapping, update: &StructuredUpdate) -> Result<()> {
    if let Some(mode) = &update.mode {
        require_enum("mode", mode, &["rule", "global", "direct"])?;
        map.insert(
            YamlValue::String("mode".to_string()),
            YamlValue::String(mode.clone()),
        );
    }
    if let Some(log_level) = &update.log_level {
        require_enum(
            "log-level",
            log_level,
            &["silent", "error", "warning", "info", "debug"],
        )?;
        map.insert(
            YamlValue::String("log-level".to_string()),
            YamlValue::String(log_level.clone()),
        );
    }
    if let Some(value) = update.allow_lan {
        map.insert(
            YamlValue::String("allow-lan".to_string()),
            YamlValue::Bool(value),
        );
    }
    if let Some(value) = &update.external_controller {
        require_clean("external-controller", value)?;
        map.insert(
            YamlValue::String("external-controller".to_string()),
            YamlValue::String(value.clone()),
        );
    }
    if let Some(value) = &update.external_ui {
        require_clean("external-ui", value)?;
        map.insert(
            YamlValue::String("external-ui".to_string()),
            YamlValue::String(value.clone()),
        );
    }
    if let Some(value) = update.profile_store_selected {
        profile_map(map)?.insert(
            YamlValue::String("store-selected".to_string()),
            YamlValue::Bool(value),
        );
    }
    if let Some(value) = update.profile_store_fake_ip {
        profile_map(map)?.insert(
            YamlValue::String("store-fake-ip".to_string()),
            YamlValue::Bool(value),
        );
    }
    if let Some(value) = update.unified_delay {
        map.insert(
            YamlValue::String("unified-delay".to_string()),
            YamlValue::Bool(value),
        );
    }
    if let Some(value) = update.tcp_concurrent {
        map.insert(
            YamlValue::String("tcp-concurrent".to_string()),
            YamlValue::Bool(value),
        );
    }
    if let Some(value) = &update.find_process_mode {
        require_enum("find-process-mode", value, &["always", "strict", "off"])?;
        map.insert(
            YamlValue::String("find-process-mode".to_string()),
            YamlValue::String(value.clone()),
        );
    }
    Ok(())
}

fn profile_map(map: &mut Mapping) -> Result<&mut Mapping> {
    let key = YamlValue::String("profile".to_string());
    if !map.contains_key(&key) {
        map.insert(key.clone(), YamlValue::Mapping(Mapping::new()));
    }
    match map.get_mut(&key) {
        Some(YamlValue::Mapping(profile)) => Ok(profile),
        Some(_) => Err(BoxError::validation("profile must be a mapping")),
        None => Err(BoxError::internal("profile insertion failed")),
    }
}

fn string_value(map: &Mapping, key: &str, default: &str, allowed: &[&str]) -> Result<String> {
    let key_value = YamlValue::String(key.to_string());
    let value = match map.get(&key_value) {
        Some(YamlValue::String(value)) => {
            if !allowed.is_empty() {
                value.to_lowercase()
            } else {
                // Preserve exact case for free-form strings like external-ui / external-controller.
                value.clone()
            }
        }
        Some(_) => {
            return Err(BoxError::validation(format!("{key} must be a string")));
        }
        None => default.to_string(),
    };
    if !allowed.is_empty() {
        require_enum(key, &value, allowed)?;
    }
    Ok(value)
}

fn bool_value(map: &Mapping, key: &str, default: bool) -> Result<bool> {
    match map.get(YamlValue::String(key.to_string())) {
        Some(YamlValue::Bool(value)) => Ok(*value),
        Some(_) => Err(BoxError::validation(format!("{key} must be a boolean"))),
        None => Ok(default),
    }
}

fn profile_bool(map: &Mapping, key: &str, default: bool) -> Result<bool> {
    let Some(YamlValue::Mapping(profile)) = map.get(YamlValue::String("profile".to_string()))
    else {
        return Ok(default);
    };
    match profile.get(YamlValue::String(key.to_string())) {
        Some(YamlValue::Bool(value)) => Ok(*value),
        Some(_) => Err(BoxError::validation(format!(
            "profile.{key} must be a boolean"
        ))),
        None => Ok(default),
    }
}

fn require_enum(key: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(BoxError::validation(format!(
            "{key} has an unsupported value: {value}"
        )))
    }
}

fn require_clean(key: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(BoxError::validation(format!(
            "{key} contains a control character"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply, get, ApplyParams, StructuredUpdate};
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
        let root =
            std::env::temp_dir().join(format!("boxctl-core-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("mihomo")).expect("mkdir");
        fs::copy(
            repo_root().join("box/mihomo/config.yaml"),
            root.join("mihomo/config.yaml"),
        )
        .expect("copy config");
        root
    }

    #[test]
    fn get_returns_structured_and_raw() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let data = get(&paths).expect("get");
        assert_eq!(data["core"], "mihomo");
        assert!(data["raw"].as_str().expect("raw").contains("mixed-port"));
        assert_eq!(data["structured"]["mode"], "rule");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn structured_apply_changes_only_supplied_fields() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let result = apply(
            &paths,
            ApplyParams {
                raw: None,
                structured: Some(StructuredUpdate {
                    mode: Some("global".to_string()),
                    log_level: Some("info".to_string()),
                    allow_lan: Some(false),
                    external_controller: None,
                    external_ui: None,
                    profile_store_selected: None,
                    profile_store_fake_ip: None,
                    unified_delay: None,
                    tcp_concurrent: None,
                    find_process_mode: None,
                }),
            },
        )
        .expect("apply");
        assert_eq!(result["mode"], "structured");
        let after = get(&paths).expect("get");
        assert_eq!(after["structured"]["mode"], "global");
        assert_eq!(after["structured"]["log-level"], "info");
        assert_eq!(after["structured"]["allow-lan"], false);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_raw_does_not_change_file() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let before = fs::read(&paths.config).expect("before");
        let error = apply(
            &paths,
            ApplyParams {
                raw: Some("not: [valid".to_string()),
                structured: None,
            },
        )
        .expect_err("invalid yaml");
        assert_eq!(error.code, crate::error::ErrorCode::Validation);
        assert_eq!(fs::read(&paths.config).expect("after"), before);
        let _ = fs::remove_dir_all(root);
    }
}
