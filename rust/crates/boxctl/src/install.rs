use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;

pub fn prepare_upgrade(paths: &Paths) -> Result<Value> {
    if !paths.settings.is_file() || !paths.defaults.is_file() {
        return Err(BoxError::new(
            ErrorCode::NotFound,
            "settings.json or settings.default.json is missing; refusing to reset",
        ));
    }
    let backup = unique_backup_dir(paths)?;
    util::ensure_dir(&backup, true)?;
    let mut files = Vec::new();
    for source in [
        &paths.settings,
        &paths.token,
        &paths.home.join("settings.ini"),
        &paths.home.join("crontab.cfg"),
    ] {
        if source.is_file() {
            let name = source.file_name().unwrap_or_default();
            let target = backup.join(name);
            copy_private(source, &target)?;
            files.push(target.display().to_string());
        }
    }
    let log_dir = backup.join("logs");
    util::ensure_dir(&log_dir, true)?;
    for source in [&paths.runs_log, &paths.tool_log, &paths.core_log] {
        if source.is_file() {
            let name = source.file_name().unwrap_or_default();
            let target = log_dir.join(name);
            copy_private(source, &target)?;
            files.push(target.display().to_string());
        }
    }
    write_manifest(&backup, &files)?;

    let defaults = fs::read_to_string(&paths.defaults)
        .map_err(|error| BoxError::from(error).with_detail("cannot read settings defaults"))?;
    let mut settings: Map<String, Value> = serde_json::from_str(&defaults)
        .map_err(|_| BoxError::new(ErrorCode::Content, "settings.default.json is invalid"))?;
    settings.insert(
        "boot_auto_start".to_string(),
        Value::String("false".to_string()),
    );
    util::atomic_write(
        &paths.settings,
        serde_json::to_vec_pretty(&Value::Object(settings))
            .map_err(|_| BoxError::internal("cannot serialize reset settings"))?
            .as_slice(),
        0o600,
    )?;
    let _ = util::remove_file_if_exists(&paths.token);
    let _ = util::remove_file_if_exists(&paths.package_list);
    let _ = util::remove_file_if_exists(&paths.gid_list);
    let _ = util::remove_file_if_exists(&paths.ap_list);
    let upgrade = json!({
        "required": true,
        "confirmed": false,
        "from": "legacy",
        "to": env!("CARGO_PKG_VERSION")
    });
    util::atomic_write(
        &paths.upgrade_state,
        serde_json::to_vec_pretty(&upgrade)
            .map_err(|_| BoxError::internal("cannot serialize upgrade state"))?
            .as_slice(),
        0o600,
    )?;
    Ok(json!({
        "backup": backup.display().to_string(),
        "files": files,
        "settings_reset": true,
        "token_cleared": true,
        "old_lists_reset": true
    }))
}

fn unique_backup_dir(paths: &Paths) -> Result<PathBuf> {
    let base = format!("upgrade-{}", util::now_ms() / 1000);
    let mut candidate = paths.backup_dir.join(&base);
    let mut index = 2;
    while candidate.exists() {
        candidate = paths.backup_dir.join(format!("{base}-{index}"));
        index += 1;
    }
    Ok(candidate)
}

fn copy_private(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        util::ensure_dir(parent, true)?;
    }
    fs::copy(source, target).map_err(|error| {
        BoxError::from(error).with_detail(format!("cannot back up {}", source.display()))
    })?;
    fs::set_permissions(target, fs::Permissions::from_mode(0o600))
        .map_err(|error| BoxError::from(error).with_detail("cannot chmod backup file"))?;
    Ok(())
}

fn write_manifest(backup: &Path, files: &[String]) -> Result<()> {
    let manifest = json!({
        "created_at": util::format_unix_utc(util::now_ms() / 1000),
        "files": files
    });
    util::atomic_write(
        &backup.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|_| BoxError::internal("cannot serialize backup manifest"))?
            .as_slice(),
        0o600,
    )
}

#[cfg(test)]
mod tests {
    use super::prepare_upgrade;
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

    fn root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-install-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("state")).expect("state");
        let repo = repo_root();
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
        let mut settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(repo.join("box/settings.default.json")).expect("read defaults"),
        )
        .expect("parse defaults");
        settings["boot_auto_start"] = serde_json::json!("true");
        fs::write(
            root.join("settings.json"),
            serde_json::to_vec_pretty(&settings).expect("serialize"),
        )
        .expect("settings");
        fs::write(root.join("githubtoken"), "secret").expect("token");
        fs::write(root.join("package.list.cfg"), "old").expect("list");
        root
    }

    #[test]
    fn upgrade_prepare_backs_up_then_resets() {
        let home = root();
        let paths = Paths::new(home.clone());
        let result = prepare_upgrade(&paths).expect("prepare");
        let backup = PathBuf::from(result["backup"].as_str().expect("backup"));
        assert!(backup.join("settings.json").is_file());
        assert!(backup.join("githubtoken").is_file());
        assert!(backup.join("manifest.json").is_file());
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(home.join("settings.json")).expect("settings"),
        )
        .expect("json");
        assert_eq!(settings["boot_auto_start"], "false");
        assert!(!home.join("githubtoken").exists());
        assert!(!home.join("package.list.cfg").exists());
        let upgrade: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(home.join("state/upgrade.json")).expect("upgrade state"),
        )
        .expect("upgrade json");
        assert_eq!(upgrade["required"], true);
        assert_eq!(upgrade["confirmed"], false);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn backup_failure_never_resets_settings() {
        let home = root();
        fs::write(home.join("backup"), b"not a directory").expect("backup blocker");
        let paths = Paths::new(home.clone());
        let error = prepare_upgrade(&paths).expect_err("backup failure");
        assert_eq!(error.code, crate::error::ErrorCode::Io);
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(home.join("settings.json")).expect("settings"),
        )
        .expect("json");
        assert_eq!(settings["boot_auto_start"], "true");
        assert!(home.join("githubtoken").is_file());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn missing_settings_never_resets() {
        let home = root();
        fs::remove_file(home.join("settings.json")).expect("remove");
        let paths = Paths::new(home.clone());
        let error = prepare_upgrade(&paths).expect_err("missing");
        assert_eq!(error.code, crate::error::ErrorCode::NotFound);
        assert!(home.join("githubtoken").exists());
        let _ = fs::remove_dir_all(home);
    }
}
