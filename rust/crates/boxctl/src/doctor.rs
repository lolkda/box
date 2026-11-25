use std::os::unix::fs::PermissionsExt;

use serde_json::{json, Value};

use crate::error::Result;
use std::path::Path;

use crate::network;
use crate::paths::Paths;
use crate::settings::SettingsStore;
use crate::upgrade;

pub fn check(paths: &Paths, daemon: bool) -> Result<Value> {
    let mut checks = Vec::new();
    let mut warnings = Vec::new();

    match SettingsStore::open(paths.clone()).and_then(|store| store.load()) {
        Ok(settings) => {
            checks.push(item("settings", "pass", "settings load and validate"));
            let mode = settings
                .get("network_mode")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let capabilities = network::detect(paths);
            if mode == "tun" && !capabilities.tun {
                checks.push(item("tun", "fail", "TUN device is unavailable"));
                warnings.push(
                    json!({"code": "E_TUN_UNAVAILABLE", "message": "TUN device is unavailable"}),
                );
            } else if mode != "tun" && !capabilities.iptables {
                checks.push(item("iptables", "warn", "iptables is unavailable"));
                warnings.push(
                    json!({"code": "E_IPTABLES_UNAVAILABLE", "message": "iptables is unavailable"}),
                );
            } else {
                checks.push(item("network", "pass", format!("mode={mode}")));
            }
            if mode == "tun" {
                if let Some(private_mode) = android_private_dns_mode() {
                    if private_mode != "off" {
                        checks.push(item(
                            "dns",
                            "warn",
                            format!("Android private DNS is {private_mode}; it can bypass TUN DNS hijack"),
                        ));
                        warnings.push(json!({
                            "code": "E_DNS_PRIVATE",
                            "message": "Android private DNS can bypass TUN DNS hijack"
                        }));
                    }
                }
            }
        }
        Err(error) => {
            checks.push(item("settings", "fail", error.message.clone()));
            warnings.push(json!({"code": error.code.as_str(), "message": error.message}));
        }
    }

    if paths.token.is_file() {
        checks.push(item("token", "pass", "github token is configured"));
    } else {
        checks.push(item("token", "warn", "github token is not configured"));
        warnings.push(json!({
            "code": "E_TOKEN_MISSING",
            "message": "github token is not configured"
        }));
    }

    match std::fs::metadata(&paths.core_bin) {
        Ok(metadata) if metadata.permissions().mode() & 0o111 != 0 => {
            checks.push(item("core", "pass", paths.core_bin.display().to_string()));
        }
        Ok(_) => {
            checks.push(item("core", "fail", "core binary is not executable"));
            warnings.push(json!({"code": "E_CORE_NOT_EXECUTABLE", "message": "core binary is not executable"}));
        }
        Err(_) => {
            checks.push(item("core", "fail", "core binary is missing"));
            warnings.push(json!({"code": "E_CORE_MISSING", "message": "core binary is missing"}));
        }
    }

    let startup_hooks = [
        Path::new("/data/adb/ksu/service.d/box_service.sh"),
        Path::new("/data/adb/service.d/box_service.sh"),
    ];
    if let Some(hook) = startup_hooks.iter().find(|path| path.is_file()) {
        checks.push(item("startup", "pass", hook.display().to_string()));
    } else {
        checks.push(item(
            "startup",
            "warn",
            "box_service.sh startup hook was not found",
        ));
        warnings.push(json!({
            "code": "E_STARTUP_HOOK_MISSING",
            "message": "box_service.sh startup hook was not found"
        }));
    }

    match crate::yaml::read_map(&paths.config) {
        Ok(_) => checks.push(item("config", "pass", "config.yaml is valid YAML")),
        Err(error) => {
            checks.push(item("config", "fail", error.message.clone()));
            warnings.push(json!({"code": error.code.as_str(), "message": error.message}));
        }
    }

    if let Some(certs) = crate::download::system_root_certs() {
        checks.push(item(
            "tls",
            "pass",
            format!("android system root store ({} certs)", certs.len()),
        ));
    } else {
        checks.push(item("tls", "pass", "bundled Mozilla roots fallback"));
    }

    let upgrade = upgrade::status(paths)?;
    if upgrade["required"].as_bool().unwrap_or(false)
        && !upgrade["confirmed"].as_bool().unwrap_or(false)
    {
        checks.push(item("upgrade", "warn", "upgrade confirmation is required"));
        warnings.push(
            json!({"code": "E_NEEDS_CONFIRMATION", "message": "upgrade confirmation is required"}),
        );
    } else {
        checks.push(item("upgrade", "pass", "no pending upgrade confirmation"));
    }

    let failed = checks.iter().any(|check| check["status"] == "fail");
    let warned = checks.iter().any(|check| check["status"] == "warn");
    Ok(json!({
        "daemon": daemon,
        "degraded": failed || warned,
        "checks": checks,
        "warnings": warnings
    }))
}

fn item(name: &str, status: &str, detail: impl Into<String>) -> Value {
    json!({"name": name, "status": status, "detail": detail.into()})
}

fn android_private_dns_mode() -> Option<String> {
    let output = std::process::Command::new("settings")
        .args(["get", "global", "private_dns_mode"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::check;
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn check_reports_missing_core_and_config_without_claiming_pass() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-doctor-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("state")).expect("state");
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..");
        fs::copy(
            repo.join("box/settings.default.json"),
            root.join("settings.default.json"),
        )
        .expect("defaults");
        fs::copy(
            repo.join("box/settings.schema.json"),
            root.join("settings.schema.json"),
        )
        .expect("schema");
        let paths = Paths::new(root.clone());
        let data = check(&paths, false).expect("doctor");
        assert_eq!(data["daemon"], false);
        let checks = data["checks"].as_array().expect("checks");
        assert!(checks
            .iter()
            .any(|item| item["name"] == "core" && item["status"] == "fail"));
        assert!(checks
            .iter()
            .any(|item| item["name"] == "config" && item["status"] == "fail"));
        let _ = fs::remove_dir_all(root);
    }
}
