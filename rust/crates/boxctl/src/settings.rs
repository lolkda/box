use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;

const SETTINGS_MODE: u32 = 0o600;
const TOKEN_MODE: u32 = 0o600;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyParams {
    pub settings: Map<String, Value>,
    #[serde(default)]
    pub githubtoken: Option<String>,
    #[serde(default)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub changed: Vec<String>,
    pub revision: u64,
}

enum TokenUpdate {
    Keep,
    Clear,
    Set(String),
}

pub struct SettingsStore {
    paths: Paths,
    defaults: Map<String, Value>,
    revision: u64,
}

impl SettingsStore {
    pub fn open(paths: Paths) -> Result<Self> {
        paths.ensure_home()?;
        let defaults = util::read_json_object(&paths.defaults)?;
        let schema = util::read_json_object(&paths.schema)?;
        let schema_keys = schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                BoxError::validation("settings.schema.json has no properties object")
                    .with_detail(paths.schema.display().to_string())
            })?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let default_keys = defaults.keys().cloned().collect::<BTreeSet<_>>();
        if schema_keys != default_keys {
            return Err(BoxError::validation(
                "settings.default.json keys do not match settings.schema.json",
            )
            .with_detail(format!(
                "defaults={} schema={}",
                default_keys.len(),
                schema_keys.len()
            )));
        }
        let revision = read_revision(&paths)?;
        let store = Self {
            paths,
            defaults,
            revision,
        };
        store.validate_map(&store.defaults)?;
        Ok(store)
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn bump_revision(&mut self) -> Result<u64> {
        let next = self.revision.saturating_add(1);
        write_revision(&self.paths, next)?;
        self.revision = next;
        Ok(self.revision)
    }

    pub fn load(&self) -> Result<Map<String, Value>> {
        let mut current = match util::read_optional(&self.paths.settings)? {
            Some(bytes) => parse_object(&bytes, &self.paths.settings)?,
            None => Map::new(),
        };
        for key in current.keys() {
            if !self.defaults.contains_key(key) {
                return Err(BoxError::validation(format!("unknown setting key: {key}")));
            }
        }
        for (key, value) in &self.defaults {
            current.entry(key.clone()).or_insert_with(|| value.clone());
        }
        // eBPF support was removed. Migrate old persisted values instead of
        // bricking the daemon on an otherwise valid settings.json.
        let mut migrated = false;
        if current.get("network_mode").and_then(Value::as_str) == Some("ebpf") {
            current.insert(
                "network_mode".to_string(),
                Value::String("tproxy".to_string()),
            );
            migrated = true;
        }
        if current.get("cnip_mode").and_then(Value::as_str) == Some("ebpf") {
            current.insert("cnip_mode".to_string(), Value::String("ipset".to_string()));
            migrated = true;
        }
        if migrated {
            let bytes =
                serde_json::to_vec_pretty(&Value::Object(current.clone())).map_err(|error| {
                    BoxError::internal("cannot serialize migrated settings")
                        .with_detail(error.to_string())
                })?;
            util::atomic_write(&self.paths.settings, &bytes, 0o600)?;
        }
        self.validate_map(&current)?;
        Ok(current)
    }

    pub fn get(&self) -> Result<Value> {
        let current = self.load()?;
        let mut data = current;
        let token_set = token_is_set(&self.paths.token)?;
        data.insert(
            "sensitive".to_string(),
            serde_json::json!({"githubtoken": {"set": token_set}}),
        );
        Ok(Value::Object(data))
    }

    pub fn apply(&mut self, params: ApplyParams) -> Result<ApplyOutcome> {
        self.check_revision(params.if_revision)?;
        let current = self.load()?;
        let mut next = current.clone();
        for (key, value) in &params.settings {
            if !self.defaults.contains_key(key) {
                return Err(BoxError::validation(format!("unknown setting key: {key}")));
            }
            next.insert(key.clone(), value.clone());
        }
        self.validate_map(&next)?;

        let mut changed = self
            .defaults
            .keys()
            .filter(|key| current.get(*key) != next.get(*key))
            .cloned()
            .collect::<Vec<_>>();

        let token_update = match params.githubtoken {
            None => TokenUpdate::Keep,
            Some(value) if value.is_empty() => TokenUpdate::Clear,
            Some(value) => {
                validate_token(&value)?;
                TokenUpdate::Set(value)
            }
        };
        let token_was_set = token_is_set(&self.paths.token)?;
        let token_changed = match &token_update {
            TokenUpdate::Keep => false,
            TokenUpdate::Clear => token_was_set,
            TokenUpdate::Set(_) => true,
        };
        if changed.is_empty() && !token_changed {
            return Ok(ApplyOutcome {
                changed,
                revision: self.revision,
            });
        }

        let old_settings = util::read_optional(&self.paths.settings)?;
        if !changed.is_empty() {
            let bytes = settings_bytes(&next)?;
            util::atomic_write(&self.paths.settings, &bytes, SETTINGS_MODE)?;
        }

        let token_result = match token_update {
            TokenUpdate::Keep => Ok(()),
            TokenUpdate::Clear => util::remove_file_if_exists(&self.paths.token),
            TokenUpdate::Set(value) => {
                util::atomic_write(&self.paths.token, value.as_bytes(), TOKEN_MODE)
            }
        };
        if let Err(error) = token_result {
            if !changed.is_empty() {
                let restore = restore_settings(&self.paths.settings, old_settings.as_deref());
                if let Err(restore_error) = restore {
                    return Err(error.clone().with_detail(format!(
                        "{}; restoring settings also failed: {}",
                        error.detail.as_deref().unwrap_or("token update failed"),
                        restore_error
                    )));
                }
            }
            return Err(error.with_detail("settings were restored after token update failed"));
        }

        if token_changed {
            changed.push("githubtoken".to_string());
        }
        let next_revision = self.revision.saturating_add(1);
        write_revision(&self.paths, next_revision)?;
        self.revision = next_revision;
        Ok(ApplyOutcome {
            changed,
            revision: self.revision,
        })
    }

    pub fn set(&mut self, params: SetParams) -> Result<ApplyOutcome> {
        self.check_revision(params.if_revision)?;
        if params.key == "githubtoken" {
            return self.apply(ApplyParams {
                settings: Map::new(),
                githubtoken: Some(params.value),
                if_revision: None,
            });
        }
        let Some(default) = self.defaults.get(&params.key) else {
            return Err(BoxError::validation(format!(
                "unknown setting key: {}",
                params.key
            )));
        };
        if default.is_array() {
            return Err(BoxError::usage(
                "settings.set accepts scalar keys only; use settings.apply for arrays",
            ));
        }
        let mut settings = Map::new();
        settings.insert(params.key, Value::String(params.value));
        self.apply(ApplyParams {
            settings,
            githubtoken: None,
            if_revision: None,
        })
    }

    fn check_revision(&self, if_revision: Option<u64>) -> Result<()> {
        if let Some(expected) = if_revision {
            if expected != self.revision {
                return Err(BoxError::new(ErrorCode::Conflict, "revision mismatch")
                    .with_detail(format!("expected={expected} current={}", self.revision)));
            }
        }
        Ok(())
    }

    fn validate_map(&self, map: &Map<String, Value>) -> Result<()> {
        for (key, value) in map {
            let Some(default) = self.defaults.get(key) else {
                return Err(BoxError::validation(format!("unknown setting key: {key}")));
            };
            if default.is_string() {
                let Some(text) = value.as_str() else {
                    return Err(BoxError::validation(format!("{key} must be a string")));
                };
                validate_string(key, text)?;
            } else if default.is_array() {
                let Some(items) = value.as_array() else {
                    return Err(BoxError::validation(format!("{key} must be an array")));
                };
                for item in items {
                    let Some(text) = item.as_str() else {
                        return Err(BoxError::validation(format!(
                            "{key} must contain only strings"
                        )));
                    };
                    validate_array_item(key, text)?;
                }
            } else {
                return Err(BoxError::internal(format!(
                    "default value for {key} has unsupported type"
                )));
            }
        }
        for key in self.defaults.keys() {
            if !map.contains_key(key) {
                return Err(BoxError::validation(format!("missing setting key: {key}")));
            }
        }
        Ok(())
    }
}

fn parse_object(bytes: &[u8], path: &Path) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        BoxError::validation(format!("{} is not valid JSON", path.display()))
            .with_detail(error.to_string())
    })?;
    value.as_object().cloned().ok_or_else(|| {
        BoxError::validation(format!("{} must contain a JSON object", path.display()))
    })
}

fn settings_bytes(map: &Map<String, Value>) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(map.clone())).map_err(|error| {
        BoxError::internal("cannot serialize settings").with_detail(error.to_string())
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn restore_settings(path: &Path, old: Option<&[u8]>) -> Result<()> {
    match old {
        Some(bytes) => util::atomic_write(path, bytes, SETTINGS_MODE),
        None => util::remove_file_if_exists(path),
    }
}

fn read_revision(paths: &Paths) -> Result<u64> {
    let Some(text) = util::read_to_string_optional(&paths.revision)? else {
        return Ok(0);
    };
    text.trim()
        .parse::<u64>()
        .map_err(|_| BoxError::validation("state/revision is not a non-negative integer"))
}

fn write_revision(paths: &Paths, revision: u64) -> Result<()> {
    paths.ensure_state_dir()?;
    util::atomic_write(&paths.revision, format!("{revision}\n").as_bytes(), 0o600)
}

fn token_is_set(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len() > 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(BoxError::from(error).with_detail(format!("cannot stat {}", path.display())))
        }
    }
}

fn validate_token(value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(BoxError::validation(
            "githubtoken contains a control character",
        ));
    }
    Ok(())
}

fn validate_string(key: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(BoxError::validation(format!(
            "{key} contains a control character"
        )));
    }
    match key {
        "auto_modify_config"
        | "boot_auto_start"
        | "ipv6"
        | "performance_mode"
        | "proxy_tcp"
        | "proxy_udp"
        | "dns_hijack_tcp"
        | "dns_hijack_udp"
        | "bypass_cn_ip"
        | "bypass_cn_ip_v4"
        | "bypass_cn_ip_v6"
        | "update_subscription"
        | "update_geo"
        | "use_ghproxy"
        | "run_crontab"
        | "cgroup_memcg"
        | "taskset_cpu"
        | "cgroup_blkio"
        | "mac_filter"
        | "subscription_keep_last_good"
        | "enable_network_service_control"
        | "use_module_on_wifi"
        | "use_module_on_wifi_disconnect"
        | "use_ssid_matching"
        | "inotify_log_enabled" => require_enum(key, value, &["true", "false"])?,
        "quic" | "mihomo_dns_forward" | "mihomo_stable" => {
            require_enum(key, value, &["enable", "disable"])?
        }
        "network_mode" => require_enum(
            key,
            value,
            &["tun", "tproxy", "redirect", "mixed", "enhance"],
        )?,
        "subscription_mode" => require_enum(key, value, &["auto", "provider"])?,
        "proxy_mode" => require_enum(key, value, &["core", "blacklist", "whitelist"])?,
        "dns_hijack_mode" => require_enum(key, value, &["disable", "tproxy", "redirect"])?,
        "cnip_mode" => require_enum(key, value, &["ipset"])?,
        "mac_mode" | "use_wifi_list_mode" => require_enum(key, value, &["blacklist", "whitelist"])?,
        "tproxy_port" | "redir_port" => {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(BoxError::validation(format!(
                    "{key} must be a decimal port"
                )));
            }
            let port = value
                .parse::<u16>()
                .map_err(|_| BoxError::validation(format!("{key} is out of range")))?;
            if port == 0 {
                return Err(BoxError::validation(format!("{key} must be 1..=65535")));
            }
        }
        "weight" => {
            if value.is_empty() {
                return Ok(());
            }
            if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(BoxError::validation(
                    "weight must be empty or a decimal integer",
                ));
            }
            let weight = value
                .parse::<u32>()
                .map_err(|_| BoxError::validation("weight is out of range"))?;
            if !(1..=1000).contains(&weight) {
                return Err(BoxError::validation("weight must be 1..=1000"));
            }
        }
        "memcg_limit" => {
            if !valid_memcg_limit(value) {
                return Err(BoxError::validation(
                    "memcg_limit must look like 100M, 1024K, or 1G",
                ));
            }
        }
        "allow_cpu" => {
            if !valid_cpu_list(value) {
                return Err(BoxError::validation("allow_cpu must look like 0-3,0,5-7"));
            }
        }
        "interva_update" => {
            if value.split_whitespace().count() != 5 {
                return Err(BoxError::validation(
                    "interva_update must have five whitespace-separated fields",
                ));
            }
        }
        "cn_ip_url" | "cn_ipv6_url" | "ruleset_prefix" => {
            if !value.is_empty() && !(value.starts_with("http://") || value.starts_with("https://"))
            {
                return Err(BoxError::validation(format!(
                    "{key} must be empty or start with http:// or https://"
                )));
            }
        }
        "url_ghproxy" => {
            if !value.is_empty() && !value.starts_with("https://") {
                return Err(BoxError::validation(
                    "url_ghproxy must be empty or start with https://",
                ));
            }
        }
        "subscription_user_agent" if value.is_empty() || value.chars().count() > 255 => {
            return Err(BoxError::validation(
                "subscription_user_agent must be 1..=255 characters",
            ));
        }
        _ => {}
    }
    Ok(())
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

fn valid_memcg_limit(value: &str) -> bool {
    let mut digits = value;
    if let Some(last) = value.chars().last() {
        if matches!(last, 'K' | 'M' | 'G' | 'k' | 'm' | 'g') {
            digits = &value[..value.len() - last.len_utf8()];
        }
    }
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_cpu_list(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    value.split(',').all(|part| {
        let mut pieces = part.split('-');
        let Some(first) = pieces.next() else {
            return false;
        };
        if first.is_empty() || !first.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        match pieces.next() {
            None => true,
            Some(second) => {
                !second.is_empty()
                    && second.bytes().all(|byte| byte.is_ascii_digit())
                    && pieces.next().is_none()
            }
        }
    })
}

fn validate_array_item(key: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(BoxError::validation(format!(
            "{key} contains a control character"
        )));
    }
    match key {
        "cnip_force_proxy_apps" => {
            if !valid_package_entry(value) {
                return Err(BoxError::validation(
                    "cnip_force_proxy_apps contains an invalid package entry",
                ));
            }
        }
        "force_proxy_cidrs" => {
            if !valid_ipv4_cidr(value) {
                return Err(BoxError::validation(
                    "force_proxy_cidrs contains an invalid IPv4 CIDR",
                ));
            }
        }
        "force_proxy_cidrs6" => {
            if !value.contains(':') || value.split_whitespace().count() != 1 {
                return Err(BoxError::validation(
                    "force_proxy_cidrs6 contains an invalid IPv6 CIDR",
                ));
            }
        }
        "macs_list" | "wifi_bssids_list" if !valid_mac(value) => {
            return Err(BoxError::validation(format!(
                "{key} contains an invalid MAC address"
            )));
        }
        _ => {}
    }
    Ok(())
}

fn valid_package_entry(value: &str) -> bool {
    let package = match value.split_once(':') {
        Some((uid, package)) => {
            !uid.is_empty()
                && uid.bytes().all(|byte| byte.is_ascii_digit())
                && !package.is_empty()
                && package
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
        }
        None => {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
        }
    };
    package
}

fn valid_ipv4_cidr(value: &str) -> bool {
    let Some((address, prefix)) = value.split_once('/') else {
        return false;
    };
    if address.is_empty() || prefix.is_empty() || !prefix.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(prefix_number) = prefix.parse::<u8>() else {
        return false;
    };
    if prefix_number > 32 {
        return false;
    }
    let octets = address.split('.').collect::<Vec<_>>();
    octets.len() == 4
        && octets.iter().all(|octet| {
            !octet.is_empty()
                && octet.bytes().all(|byte| byte.is_ascii_digit())
                && octet.parse::<u16>().is_ok_and(|number| number <= 255)
        })
}

fn valid_mac(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::{ApplyParams, SetParams, SettingsStore};
    use crate::paths::Paths;
    use serde_json::{Map, Value};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
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
            std::env::temp_dir().join(format!("boxctl-settings-{}-{suffix}", std::process::id()));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn copy_base(root: &Path) {
        let repo = repo_root();
        fs::copy(
            repo.join("box/settings.default.json"),
            root.join("settings.default.json"),
        )
        .expect("copy defaults");
        fs::copy(
            repo.join("box/settings.schema.json"),
            root.join("settings.schema.json"),
        )
        .expect("copy schema");
        fs::copy(
            repo.join("box/settings.default.json"),
            root.join("settings.json"),
        )
        .expect("copy settings");
        fs::set_permissions(
            root.join("settings.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .expect("chmod settings");
    }

    fn open_store() -> (PathBuf, SettingsStore) {
        let root = temp_root();
        copy_base(&root);
        let store = SettingsStore::open(Paths::new(root.clone())).expect("open store");
        (root, store)
    }

    #[test]
    fn defaults_match_schema_and_validate() {
        let (_root, store) = open_store();
        assert_eq!(store.defaults.len(), 52);
        assert_eq!(store.revision, 0);
        let data = store.get().expect("get");
        assert_eq!(data["network_mode"], "tun");
        assert_eq!(data["sensitive"]["githubtoken"]["set"], false);
        assert!(data["sensitive"]["githubtoken"].get("preview").is_none());
    }

    #[test]
    fn migrates_removed_ebpf_values() {
        let (root, store) = open_store();
        let mut data: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).expect("read"))
                .expect("json");
        data["network_mode"] = Value::String("ebpf".to_string());
        data["cnip_mode"] = Value::String("ebpf".to_string());
        fs::write(
            root.join("settings.json"),
            serde_json::to_vec_pretty(&data).expect("encode"),
        )
        .expect("write");
        let loaded = store.load().expect("migrated load");
        assert_eq!(loaded["network_mode"], "tproxy");
        assert_eq!(loaded["cnip_mode"], "ipset");
        let persisted: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).expect("read"))
                .expect("json");
        assert_eq!(persisted["network_mode"], "tproxy");
        assert_eq!(persisted["cnip_mode"], "ipset");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_values_without_writing() {
        let (root, mut store) = open_store();
        let before = fs::read(root.join("settings.json")).expect("read before");
        let error = store
            .apply(ApplyParams {
                settings: Map::from_iter([(
                    "tproxy_port".to_string(),
                    Value::String("0".to_string()),
                )]),
                githubtoken: None,
                if_revision: None,
            })
            .expect_err("invalid port");
        assert_eq!(error.code, crate::error::ErrorCode::Validation);
        let after = fs::read(root.join("settings.json")).expect("read after");
        assert_eq!(before, after);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_updates_revision_and_token_without_preview() {
        let (root, mut store) = open_store();
        let outcome = store
            .apply(ApplyParams {
                settings: Map::from_iter([(
                    "tproxy_port".to_string(),
                    Value::String("7777".to_string()),
                )]),
                githubtoken: Some("secret-token".to_string()),
                if_revision: Some(0),
            })
            .expect("apply");
        assert_eq!(outcome.revision, 1);
        assert!(outcome.changed.iter().any(|key| key == "tproxy_port"));
        assert!(outcome.changed.iter().any(|key| key == "githubtoken"));
        let data = store.get().expect("get");
        assert_eq!(data["tproxy_port"], "7777");
        assert_eq!(data["sensitive"]["githubtoken"]["set"], true);
        assert!(data["sensitive"]["githubtoken"].get("preview").is_none());
        assert_eq!(
            fs::read_to_string(root.join("githubtoken")).expect("token"),
            "secret-token"
        );
        assert_eq!(
            fs::metadata(root.join("settings.json"))
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_schema_gap_values() {
        let (root, mut store) = open_store();
        let before = fs::read(root.join("settings.json")).expect("read before");
        let invalid = [
            ("tproxy_port", "65536"),
            ("redir_port", "abc"),
            ("weight", "1001"),
            ("interva_update", "0 0"),
            ("subscription_user_agent", "bad\nua"),
            ("url_ghproxy", "http://mirror.invalid"),
            ("not_a_setting", "x"),
        ];
        for (key, value) in invalid {
            let error = store
                .apply(ApplyParams {
                    settings: Map::from_iter([(key.to_string(), Value::String(value.to_string()))]),
                    githubtoken: None,
                    if_revision: None,
                })
                .expect_err("invalid setting");
            assert_eq!(
                error.code,
                crate::error::ErrorCode::Validation,
                "unexpected error for {key}={value}"
            );
            let after = fs::read(root.join("settings.json")).expect("read after");
            assert_eq!(before, after, "file changed for {key}={value}");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_apply_keeps_transition_runtime() {
        let (root, mut store) = open_store();
        let run_dir = root.join("run");
        fs::create_dir_all(&run_dir).expect("run dir");
        let runtime = run_dir.join("settings.runtime.sh");
        fs::write(&runtime, b"sentinel\n").expect("runtime");
        let error = store
            .apply(ApplyParams {
                settings: Map::from_iter([(
                    "weight".to_string(),
                    Value::String("1001".to_string()),
                )]),
                githubtoken: None,
                if_revision: None,
            })
            .expect_err("invalid weight");
        assert_eq!(error.code, crate::error::ErrorCode::Validation);
        assert_eq!(fs::read(&runtime).expect("read runtime"), b"sentinel\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn token_write_failure_restores_settings() {
        let (root, mut store) = open_store();
        let _ = fs::remove_file(root.join("githubtoken"));
        fs::create_dir(root.join("githubtoken")).expect("token directory");
        let before = fs::read(root.join("settings.json")).expect("read before");
        let error = store
            .apply(ApplyParams {
                settings: Map::from_iter([(
                    "redir_port".to_string(),
                    Value::String("8888".to_string()),
                )]),
                githubtoken: Some("new-token".to_string()),
                if_revision: None,
            })
            .expect_err("token path is a directory");
        assert_eq!(error.code, crate::error::ErrorCode::Io);
        let after = fs::read(root.join("settings.json")).expect("read after");
        assert_eq!(before, after);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn set_rejects_arrays_but_sets_scalars() {
        let (root, mut store) = open_store();
        let array_error = store
            .set(SetParams {
                key: "wifi_ssids_list".to_string(),
                value: "x".to_string(),
                if_revision: None,
            })
            .expect_err("array");
        assert_eq!(array_error.code, crate::error::ErrorCode::Usage);
        let outcome = store
            .set(SetParams {
                key: "network_mode".to_string(),
                value: "tproxy".to_string(),
                if_revision: None,
            })
            .expect("set");
        assert!(outcome.changed.iter().any(|key| key == "network_mode"));
        let _ = fs::remove_dir_all(root);
    }
}
