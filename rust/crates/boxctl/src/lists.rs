use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{BoxError, Result};
use crate::paths::Paths;
use crate::util;

const LIST_MODE: u32 = 0o600;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageEntry {
    #[serde(default)]
    pub uid: Option<String>,
    pub package: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfacesPayload {
    pub allow: Option<Vec<String>>,
    pub ignore: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyParams {
    pub package: Option<Vec<PackageEntry>>,
    pub gid: Option<Vec<String>>,
    pub interfaces: Option<InterfacesPayload>,
}

struct Plan {
    path: PathBuf,
    old: Option<Vec<u8>>,
    new: Vec<u8>,
    changed: &'static str,
}

pub fn get(paths: &Paths) -> Result<Value> {
    let package = parse_package(&read_optional_text(&paths.package_list)?);
    let gid = parse_gid(&read_optional_text(&paths.gid_list)?);
    let interfaces = parse_interfaces(&read_optional_text(&paths.ap_list)?);
    Ok(json!({
        "package": package
            .into_iter()
            .map(|entry| json!({
                "uid": entry.uid.unwrap_or_default(),
                "package": entry.package
            }))
            .collect::<Vec<_>>(),
        "gid": gid,
        "interfaces": interfaces,
        "files": {
            "package": paths.package_list.display().to_string(),
            "gid": paths.gid_list.display().to_string(),
            "interfaces": paths.ap_list.display().to_string()
        }
    }))
}

pub fn apply(paths: &Paths, params: ApplyParams) -> Result<Value> {
    let mut plans = Vec::new();
    if let Some(package) = &params.package {
        let content = package_content(package)?;
        plans.push(make_plan(&paths.package_list, content, "package")?);
    }
    if let Some(gid) = &params.gid {
        let content = gid_content(gid)?;
        plans.push(make_plan(&paths.gid_list, content, "gid")?);
    }
    if let Some(interfaces) = &params.interfaces {
        let content = interfaces_content(interfaces)?;
        plans.push(make_plan(&paths.ap_list, content, "interfaces")?);
    }

    let mut changed = Vec::new();
    let mut writes: Vec<&Plan> = Vec::new();
    for plan in &plans {
        if plan.old.as_deref() != Some(plan.new.as_slice()) {
            if let Err(error) = util::atomic_write(&plan.path, &plan.new, LIST_MODE) {
                for written in writes.iter().rev() {
                    restore(written);
                }
                return Err(error.with_detail("lists write failed; previous writes restored"));
            }
            writes.push(plan);
            changed.push(plan.changed);
        }
    }
    Ok(json!({"changed": changed}))
}

fn make_plan(path: &Path, new: Vec<u8>, changed: &'static str) -> Result<Plan> {
    let old = util::read_optional(path)?;
    Ok(Plan {
        path: path.to_path_buf(),
        old,
        new,
        changed,
    })
}

fn restore(plan: &Plan) {
    match &plan.old {
        Some(bytes) => {
            let _ = util::atomic_write(&plan.path, bytes, LIST_MODE);
        }
        None => {
            let _ = util::remove_file_if_exists(&plan.path);
        }
    }
}

fn read_optional_text(path: &Path) -> Result<String> {
    util::read_to_string_optional(path).map(Option::unwrap_or_default)
}

fn parse_package(raw: &str) -> Vec<PackageEntry> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("mode:") {
                return None;
            }
            let (uid, package) = match line.split_once(':') {
                Some((uid, package)) => (Some(uid.to_string()), package),
                None => (None, line),
            };
            if package.is_empty() {
                None
            } else {
                Some(PackageEntry {
                    uid,
                    package: package.to_string(),
                })
            }
        })
        .collect()
}

fn parse_gid(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn parse_interfaces(raw: &str) -> Value {
    let mut allow = Vec::new();
    let mut ignore = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let Some(action) = parts.next() else {
            continue;
        };
        let Some(interface) = parts.next() else {
            continue;
        };
        match action {
            "allow" => allow.push(interface.to_string()),
            "ignore" => ignore.push(interface.to_string()),
            _ => {}
        }
    }
    json!({"allow": allow, "ignore": ignore})
}

fn package_content(entries: &[PackageEntry]) -> Result<Vec<u8>> {
    let mut text = String::from("# 应用包名配置文件\n# 格式: <用户ID>:<包名> 或 <包名>\n");
    for entry in entries {
        validate_package(entry)?;
        match entry.uid.as_deref().filter(|uid| !uid.is_empty()) {
            Some(uid) => {
                text.push_str(uid);
                text.push(':');
                text.push_str(&entry.package);
                text.push('\n');
            }
            None => {
                text.push_str(&entry.package);
                text.push('\n');
            }
        }
    }
    Ok(text.into_bytes())
}

fn gid_content(entries: &[String]) -> Result<Vec<u8>> {
    let mut text = String::from("# GID 配置文件\n# 每行一个 GID 数字\n");
    for entry in entries {
        if entry.is_empty() || !entry.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(BoxError::validation(format!("invalid GID: {entry}")));
        }
        text.push_str(entry);
        text.push('\n');
    }
    Ok(text.into_bytes())
}

fn interfaces_content(interfaces: &InterfacesPayload) -> Result<Vec<u8>> {
    let mut text =
        String::from("# 网络接口 allow/ignore 配置\n# 格式: allow <接口名> 或 ignore <接口名>\n");
    for (action, entries) in [
        ("allow", interfaces.allow.as_deref().unwrap_or(&[])),
        ("ignore", interfaces.ignore.as_deref().unwrap_or(&[])),
    ] {
        for interface in entries {
            validate_interface(interface)?;
            text.push_str(action);
            text.push(' ');
            text.push_str(interface);
            text.push('\n');
        }
    }
    Ok(text.into_bytes())
}

fn validate_package(entry: &PackageEntry) -> Result<()> {
    if entry.package.is_empty()
        || entry
            .package
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_')))
        || !entry.package.contains('.')
        || entry.package.starts_with('.')
        || entry.package.ends_with('.')
        || entry.package.contains("..")
    {
        return Err(BoxError::validation(format!(
            "invalid package name: {}",
            entry.package
        )));
    }
    if let Some(uid) = entry.uid.as_deref().filter(|uid| !uid.is_empty()) {
        if !uid.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(BoxError::validation(format!("invalid UID: {uid}")));
        }
    }
    Ok(())
}

fn validate_interface(interface: &str) -> Result<()> {
    if interface.is_empty()
        || interface.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | ':' | '+' | '-'))
        })
    {
        return Err(BoxError::validation(format!(
            "invalid interface name: {interface}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply, get, ApplyParams, InterfacesPayload, PackageEntry};
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-lists-{}-{suffix}", std::process::id()));
        fs::create_dir_all(&root).expect("mkdir");
        root
    }

    #[test]
    fn get_parses_existing_files() {
        let root = temp_root();
        fs::write(root.join("package.list.cfg"), "0:com.example.app\n").expect("package");
        fs::write(root.join("gid.list.cfg"), "10450\n").expect("gid");
        fs::write(root.join("ap.list.cfg"), "allow wlan+\nignore wlan0\n").expect("ap");
        let paths = Paths::new(root.clone());
        let data = get(&paths).expect("get");
        assert_eq!(data["package"][0]["uid"], "0");
        assert_eq!(data["package"][0]["package"], "com.example.app");
        assert_eq!(data["gid"][0], "10450");
        assert_eq!(data["interfaces"]["allow"][0], "wlan+");
        assert_eq!(data["interfaces"]["ignore"][0], "wlan0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_replaces_selected_lists() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let result = apply(
            &paths,
            ApplyParams {
                package: Some(vec![PackageEntry {
                    uid: Some("0".to_string()),
                    package: "com.example.app".to_string(),
                }]),
                gid: Some(vec!["10450".to_string()]),
                interfaces: Some(InterfacesPayload {
                    allow: Some(vec!["wlan+".to_string()]),
                    ignore: Some(vec![]),
                }),
            },
        )
        .expect("apply");
        assert_eq!(result["changed"].as_array().expect("changed").len(), 3);
        let data = get(&paths).expect("get");
        assert_eq!(data["package"][0]["package"], "com.example.app");
        assert_eq!(data["gid"][0], "10450");
        assert_eq!(data["interfaces"]["allow"][0], "wlan+");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_package_does_not_write_any_file() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let before_package = fs::read(&paths.package_list).ok();
        let before_gid = fs::read(&paths.gid_list).ok();
        let error = apply(
            &paths,
            ApplyParams {
                package: Some(vec![PackageEntry {
                    uid: None,
                    package: "bad package".to_string(),
                }]),
                gid: Some(vec!["10450".to_string()]),
                interfaces: None,
            },
        )
        .expect_err("bad package");
        assert_eq!(error.code, crate::error::ErrorCode::Validation);
        assert_eq!(fs::read(&paths.package_list).ok(), before_package);
        assert_eq!(fs::read(&paths.gid_list).ok(), before_gid);
        let _ = fs::remove_dir_all(root);
    }
}
