use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::service;
use crate::task;
use crate::util;

const PURGE_TTL_SECONDS: u64 = 300;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeApplyParams {
    pub token: String,
    pub confirm: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForceParams {
    pub confirm: bool,
}

pub fn preflight(paths: &Paths) -> Result<Value> {
    let mut blockers = Vec::new();
    if service::status(paths)?["running"]
        .as_bool()
        .unwrap_or(false)
    {
        blockers.push(json!({"code": "CORE_RUNNING", "message": "core is running"}));
    }
    if task::running_count(paths)? > 0 {
        blockers.push(json!({"code": "TASKS_RUNNING", "message": "a task is running"}));
    }
    let can_force = !blockers.is_empty();
    Ok(json!({
        "blockers": blockers,
        "can_force": can_force,
        "report": {
            "paths": purge_targets(paths),
            "ordinary_uninstall_preserves_data": true
        }
    }))
}

pub fn apply(paths: &Paths) -> Result<Value> {
    prepare_uninstall(paths)?;
    let manager = request_manager_uninstall();
    Ok(manual_removal_result(paths, manager))
}

pub fn force(paths: &Paths, params: ForceParams) -> Result<Value> {
    if !params.confirm {
        return Err(BoxError::new(
            ErrorCode::NeedsConfirmation,
            "uninstall force requires confirm=true",
        ));
    }
    prepare_uninstall(paths)?;
    let manager = request_manager_uninstall();
    Ok(manual_removal_result(paths, manager))
}

fn prepare_uninstall(paths: &Paths) -> Result<()> {
    if service::status(paths)?["running"]
        .as_bool()
        .unwrap_or(false)
    {
        service::stop(paths)?;
    }
    if task::running_count(paths)? > 0 {
        task::interrupt_all(paths)?;
    }
    let check = preflight(paths)?;
    if !check["blockers"].as_array().is_some_and(Vec::is_empty) {
        return Err(BoxError::new(
            ErrorCode::Conflict,
            "uninstall has remaining blocking items after stop",
        ));
    }
    Ok(())
}

fn detect_manual_manager() -> Option<&'static str> {
    if Path::new("/data/adb/magisk").is_dir() || Path::new("/data/adb/magisk/magisk").is_file() {
        return Some("magisk");
    }
    if Path::new("/data/adb/apd").is_file() || Path::new("/data/adb/ap").is_dir() {
        return Some("apatch");
    }
    None
}

fn request_manager_uninstall() -> Option<Value> {
    let mut command = None;
    for candidate in ["/data/adb/ksud", "/data/adb/ksu/bin/ksud"] {
        if Path::new(candidate).is_file() {
            command = Some(candidate);
            break;
        }
    }
    if let Some(manager) = detect_manual_manager() {
        return Some(json!({
            "manager": manager,
            "removed": false,
            "hint": "remove the module from the Root manager UI"
        }));
    }
    let executable = command?;
    match Command::new(executable)
        .args(["module", "uninstall", "box_for_root"])
        .output()
    {
        Ok(output) if output.status.success() => Some(json!({
            "manager": "kernelsu",
            "removed": true
        })),
        Ok(output) => Some(json!({
            "manager": "kernelsu",
            "removed": false,
            "error": String::from_utf8_lossy(&output.stderr).trim()
        })),
        Err(error) => Some(json!({
            "manager": "kernelsu",
            "removed": false,
            "error": error.to_string()
        })),
    }
}

fn manual_removal_result(paths: &Paths, manager: Option<Value>) -> Value {
    let manager_removed = manager
        .as_ref()
        .and_then(|value| value.get("removed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "status": if manager_removed { "manager_removal_requested" } else { "manual_removal_required" },
        "message": if manager_removed {
            "core/tasks stopped and managed rules cleaned; module removal was requested from KernelSU"
        } else {
            "core/tasks stopped and managed rules cleaned; remove the module from the Root manager to finish"
        },
        "manager": manager,
        "preserved": [
            paths.settings.display().to_string(),
            paths.token.display().to_string(),
            paths.config.display().to_string(),
            paths.subscriptions.display().to_string(),
            paths.rule_profiles_dir.display().to_string(),
            paths.core_bin.display().to_string()
        ],
        "data_deleted": false
    })
}

pub fn plan(paths: &Paths) -> Result<Value> {
    let token = random_token()?;
    let now = now_seconds();
    let state = json!({
        "token": token,
        "expires_at": now + PURGE_TTL_SECONDS,
        "paths": purge_targets(paths)
    });
    util::ensure_dir(
        paths
            .purge_state
            .parent()
            .ok_or_else(|| BoxError::internal("purge state has no parent"))?,
        true,
    )?;
    util::atomic_write(
        &paths.purge_state,
        serde_json::to_vec_pretty(&state)
            .map_err(|_| BoxError::internal("cannot serialize purge plan"))?
            .as_slice(),
        0o600,
    )?;
    Ok(json!({
        "token": token,
        "expires_at": now + PURGE_TTL_SECONDS,
        "paths": purge_targets(paths)
    }))
}

pub fn apply_purge(paths: &Paths, params: PurgeApplyParams) -> Result<Value> {
    if !params.confirm {
        return Err(BoxError::new(
            ErrorCode::NeedsConfirmation,
            "purge requires confirm=true",
        ));
    }
    let Some(text) = util::read_to_string_optional(&paths.purge_state)? else {
        return Err(BoxError::new(ErrorCode::Conflict, "purge token is missing"));
    };
    let state: Value = serde_json::from_str(&text)
        .map_err(|_| BoxError::new(ErrorCode::Conflict, "purge token is invalid"))?;
    let stored = state
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if stored != params.token {
        return Err(BoxError::new(ErrorCode::Conflict, "purge token is invalid"));
    }
    let expires = state.get("expires_at").and_then(Value::as_u64).unwrap_or(0);
    if now_seconds() > expires {
        let _ = util::remove_file_if_exists(&paths.purge_state);
        return Err(BoxError::new(
            ErrorCode::Conflict,
            "purge token has expired",
        ));
    }
    let home = fs::canonicalize(&paths.home).unwrap_or_else(|_| paths.home.clone());
    let mut deleted = Vec::new();
    for path in purge_targets(paths) {
        let path = PathBuf::from(path);
        if !safe_target(&home, &path) {
            let _ = util::remove_file_if_exists(&paths.purge_state);
            return Err(BoxError::permission("purge target is outside the Box home"));
        }
        match remove_safe(&home, &path) {
            Ok(()) => {
                if !path.exists() {
                    deleted.push(path.display().to_string());
                }
            }
            Err(error) => {
                let _ = util::remove_file_if_exists(&paths.purge_state);
                return Err(error.with_detail(format!(
                    "purge failed at {}; token was consumed",
                    path.display()
                )));
            }
        }
    }
    let _ = util::remove_file_if_exists(&paths.purge_state);
    Ok(json!({"deleted": deleted, "report": {"complete": true}}))
}

fn purge_targets(paths: &Paths) -> Vec<String> {
    vec![
        paths.settings.display().to_string(),
        paths.defaults.display().to_string(),
        paths.schema.display().to_string(),
        paths.token.display().to_string(),
        paths.revision.display().to_string(),
        paths.config.display().to_string(),
        paths.package_list.display().to_string(),
        paths.gid_list.display().to_string(),
        paths.ap_list.display().to_string(),
        paths.subscriptions.display().to_string(),
        paths.subscriptions_ua_dir.display().to_string(),
        paths.providers_dir.display().to_string(),
        paths.subscriptions_providers.display().to_string(),
        paths.runs_log.display().to_string(),
        paths.tool_log.display().to_string(),
        paths.core_log.display().to_string(),
        paths.rule_profiles_dir.display().to_string(),
        paths.tasks_dir.display().to_string(),
        paths.core_bin.display().to_string(),
        paths.state_dir.display().to_string(),
    ]
}

fn safe_target(home: &Path, path: &Path) -> bool {
    if path == home || !path.starts_with(home) {
        return false;
    }
    path.components()
        .all(|component| !matches!(component, Component::ParentDir))
}

fn remove_safe(home: &Path, path: &Path) -> Result<()> {
    if !path.starts_with(home) || path == home {
        return Err(BoxError::permission("refusing to purge outside Box home"));
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(BoxError::from(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        fs::remove_file(path).map_err(BoxError::from)?;
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(BoxError::from)? {
            let entry = entry.map_err(BoxError::from)?;
            remove_safe(home, &entry.path())?;
        }
        fs::remove_dir(path).map_err(BoxError::from)?;
    }
    Ok(())
}

fn random_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = fs::File::open("/dev/urandom") {
        if file.read_exact(&mut bytes).is_ok() {
            return Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect());
        }
    }
    let fallback = format!("{}-{}", now_seconds(), std::process::id());
    Ok(fallback)
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs())
}

#[cfg(test)]
mod tests {
    use super::{apply_purge, plan, preflight, PurgeApplyParams};
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-purge-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("state")).expect("state");
        fs::create_dir_all(root.join("run")).expect("run");
        fs::create_dir_all(root.join("mihomo")).expect("mihomo");
        fs::write(root.join("settings.json"), "{}").expect("settings");
        fs::write(root.join("githubtoken"), "secret").expect("token");
        fs::write(root.join("mihomo/config.yaml"), "mode: rule\n").expect("config");
        root
    }

    #[test]
    fn purge_plan_is_confirm_and_token_gated_and_stays_inside_home() {
        let home = root();
        let parent = home.parent().expect("parent").to_path_buf();
        fs::write(parent.join("boxctl-outside-purge-sentinel"), "keep").expect("outside");
        let paths = Paths::new(home.clone());
        let planned = plan(&paths).expect("plan");
        let token = planned["token"].as_str().expect("token").to_string();
        let denied = apply_purge(
            &paths,
            PurgeApplyParams {
                token: token.clone(),
                confirm: false,
            },
        )
        .expect_err("confirm");
        assert_eq!(denied.code, crate::error::ErrorCode::NeedsConfirmation);
        let applied = apply_purge(
            &paths,
            PurgeApplyParams {
                token: token.clone(),
                confirm: true,
            },
        )
        .expect("purge");
        assert_eq!(applied["report"]["complete"], true);
        assert!(!home.join("settings.json").exists());
        assert!(parent.join("boxctl-outside-purge-sentinel").exists());
        let reused = apply_purge(
            &paths,
            PurgeApplyParams {
                token,
                confirm: true,
            },
        )
        .expect_err("reuse");
        assert_eq!(reused.code, crate::error::ErrorCode::Conflict);
        let outside = parent.join("boxctl-outside-purge-sentinel");
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn preflight_reports_no_blockers_without_core_or_tasks() {
        let home = root();
        let paths = Paths::new(home.clone());
        let value = preflight(&paths).expect("preflight");
        assert_eq!(value["blockers"].as_array().expect("blockers").len(), 0);
        let _ = fs::remove_dir_all(home);
    }
}
