use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::download::{self, Policy};
use crate::error::{BoxError, Result};
use crate::paths::Paths;
use crate::util;

pub fn yq_url() -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "arm" => "arm",
        "x86" => "386",
        other => {
            return Err(BoxError::manager_unavailable(format!(
                "unsupported architecture for yq: {other}"
            )))
        }
    };
    Ok(format!(
        "https://github.com/taamarin/yq/releases/download/prerelease/yq_android_{arch}"
    ))
}

pub fn mihomo_asset_url(tag: &str) -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64-v8",
        "x86_64" => "amd64",
        other => {
            return Err(BoxError::manager_unavailable(format!(
                "unsupported architecture for mihomo: {other}"
            )))
        }
    };
    let tag = tag.trim();
    if !tag.starts_with('v') || tag.len() < 2 {
        return Err(BoxError::validation("mihomo release tag is invalid"));
    }
    Ok(format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{tag}/mihomo-android-{arch}-{tag}.gz"
    ))
}

fn release_tag_from_page(html: &str) -> Option<String> {
    for (index, _) in html.match_indices("/releases/tag/") {
        let rest = &html[index + "/releases/tag/".len()..];
        let tag: String = rest
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric()
                    || *character == '.'
                    || *character == '-'
                    || *character == '_'
            })
            .collect();
        if tag.starts_with('v') && tag.chars().any(|character| character.is_ascii_digit()) {
            return Some(tag);
        }
        if !tag.is_empty()
            && tag
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
        {
            return Some(tag);
        }
    }
    None
}

fn latest_page_tag(paths: &Paths, url: &str, policy: &Policy) -> Result<String> {
    let page = paths.run_dir.join("release-page.html");
    let _ = util::remove_file_if_exists(&page);
    download::download_to_file(&page, url, policy, None)?;
    let html = fs::read_to_string(&page)
        .map_err(|error| BoxError::from(error).with_detail("cannot read release page"))?;
    let _ = util::remove_file_if_exists(&page);
    release_tag_from_page(&html).ok_or_else(|| {
        BoxError::new(
            crate::error::ErrorCode::Content,
            "release tag was not found on the release page",
        )
    })
}

pub fn curl_url(tag: &str) -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "amd64",
        "arm" => "armv7",
        "x86" => "i686",
        other => {
            return Err(BoxError::manager_unavailable(format!(
                "unsupported architecture for curl: {other}"
            )))
        }
    };
    let tag = tag.trim();
    if tag.is_empty()
        || !tag
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        || !tag
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.' || character == '-')
    {
        return Err(BoxError::validation("curl release tag is invalid"));
    }
    Ok(format!(
        "https://github.com/stunnel/static-curl/releases/download/{tag}/curl-linux-{arch}-glibc-{tag}.tar.xz"
    ))
}

fn find_curl(dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_curl(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|name| name == "curl") {
            return Some(path);
        }
    }
    None
}

pub fn update_curl(paths: &Paths, policy: &Policy) -> Result<Value> {
    util::ensure_dir(&paths.run_dir, true)?;
    let api = paths.run_dir.join("curl-release.json");
    let archive = paths.run_dir.join("curl.tar.xz");
    let extract = paths.run_dir.join("curl-extract");
    let _ = util::remove_file_if_exists(&api);
    let _ = util::remove_file_if_exists(&archive);
    let _ = fs::remove_dir_all(&extract);
    util::ensure_dir(&extract, true)?;

    let api_result = download::download_to_file(
        &api,
        "https://api.github.com/repos/stunnel/static-curl/releases/latest",
        policy,
        None,
    );
    let tag = match api_result {
        Ok(_) => {
            let text = fs::read_to_string(&api)
                .map_err(|error| BoxError::from(error).with_detail("cannot read curl metadata"))?;
            let metadata: Value = serde_json::from_str(&text).map_err(|_| {
                BoxError::new(crate::error::ErrorCode::Content, "invalid curl metadata")
            })?;
            metadata
                .get("tag_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    BoxError::new(crate::error::ErrorCode::Content, "curl tag_name is missing")
                })?
        }
        Err(_) => latest_page_tag(
            paths,
            "https://github.com/stunnel/static-curl/releases/latest",
            policy,
        )?,
    };
    let url = curl_url(&tag)?;
    download::download_to_file(&archive, &url, policy, None)?;

    // Android has no xz binary by default; decode in Rust before calling tar.
    let tar_path = paths.run_dir.join("curl.tar");
    let _ = util::remove_file_if_exists(&tar_path);
    {
        let mut input = BufReader::new(
            File::open(&archive)
                .map_err(|error| BoxError::from(error).with_detail("cannot open curl archive"))?,
        );
        let mut output = File::create(&tar_path)
            .map_err(|error| BoxError::from(error).with_detail("cannot create curl tar"))?;
        lzma_rs::xz_decompress(&mut input, &mut output).map_err(|error| {
            let _ = util::remove_file_if_exists(&tar_path);
            BoxError::new(
                crate::error::ErrorCode::Content,
                "cannot decompress curl archive",
            )
            .with_detail(error.to_string())
        })?;
    }

    let status = Command::new("tar")
        .arg("-xf")
        .arg(&tar_path)
        .arg("-C")
        .arg(&extract)
        .status()
        .map_err(|error| BoxError::from(error).with_detail("cannot start tar"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&extract);
        let _ = util::remove_file_if_exists(&tar_path);
        return Err(BoxError::new(
            crate::error::ErrorCode::Command,
            "tar failed to extract curl",
        ));
    }
    let binary = find_curl(&extract).ok_or_else(|| {
        BoxError::new(
            crate::error::ErrorCode::Content,
            "curl binary not found in archive",
        )
    })?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
        .map_err(|error| BoxError::from(error).with_detail("cannot chmod curl"))?;
    let target = paths.home.join("bin").join("curl");
    util::ensure_dir(
        target
            .parent()
            .ok_or_else(|| BoxError::internal("curl target has no parent"))?,
        true,
    )?;
    if target.is_file() {
        util::ensure_dir(&paths.home.join("bin").join("backup"), true)?;
        fs::copy(
            &target,
            paths.home.join("bin").join("backup").join("curl.bak"),
        )
        .map_err(|error| BoxError::from(error).with_detail("cannot back up old curl"))?;
    }
    fs::rename(&binary, &target)
        .map_err(|error| BoxError::from(error).with_detail("cannot publish curl"))?;
    let _ = util::remove_file_if_exists(&api);
    let _ = util::remove_file_if_exists(&archive);
    let _ = util::remove_file_if_exists(&tar_path);
    let _ = fs::remove_dir_all(&extract);
    Ok(json!({
        "tag": tag,
        "path": target.display().to_string()
    }))
}

pub fn parse_smart_tag(html: &str) -> Option<String> {
    for (index, _) in html.match_indices("smart-") {
        let rest = &html[index + "smart-".len()..];
        let suffix = rest
            .chars()
            .take_while(|character| character.is_ascii_hexdigit())
            .collect::<String>();
        if suffix.len() >= 7 {
            return Some(format!("smart-{suffix}"));
        }
    }
    None
}

fn install_gz_core(paths: &Paths, url: &str, policy: &Policy) -> Result<Value> {
    util::ensure_dir(&paths.run_dir, true)?;
    let archive = paths.run_dir.join("mihomo-download.gz");
    let temp_bin = paths.run_dir.join("mihomo-download.bin");
    let _ = util::remove_file_if_exists(&archive);
    let _ = util::remove_file_if_exists(&temp_bin);
    let report = download::download_to_file(&archive, url, policy, None)?;
    let mut child = Command::new("gzip")
        .arg("-dc")
        .arg(&archive)
        .stdout(Stdio::from(File::create(&temp_bin).map_err(|error| {
            BoxError::from(error).with_detail("cannot create core temp")
        })?))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| BoxError::from(error).with_detail("cannot start gzip"))?;
    let status = child
        .wait()
        .map_err(|error| BoxError::from(error).with_detail("cannot wait for gzip"))?;
    if !status.success() {
        let _ = util::remove_file_if_exists(&temp_bin);
        return Err(BoxError::new(
            crate::error::ErrorCode::Command,
            "gzip failed to decompress core",
        ));
    }
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&temp_bin, fs::Permissions::from_mode(0o755))
        .map_err(|error| BoxError::from(error).with_detail("cannot chmod core"))?;
    if paths.core_bin.is_file() {
        util::ensure_dir(&paths.home.join("bin").join("backup"), true)?;
        fs::copy(
            &paths.core_bin,
            paths.home.join("bin").join("backup").join("mihomo.bak"),
        )
        .map_err(|error| BoxError::from(error).with_detail("cannot back up old core"))?;
    }
    fs::rename(&temp_bin, &paths.core_bin)
        .map_err(|error| BoxError::from(error).with_detail("cannot publish core"))?;
    let _ = util::remove_file_if_exists(&archive);
    Ok(json!({
        "path": paths.core_bin.display().to_string(),
        "archive_bytes": report.bytes,
        "archive_sha256": report.sha256
    }))
}

pub fn update_mihomo_smart(paths: &Paths, policy: &Policy) -> Result<Value> {
    if std::env::consts::ARCH != "aarch64" {
        return Err(BoxError::manager_unavailable(
            "mihomo_smart currently requires aarch64",
        ));
    }
    util::ensure_dir(&paths.run_dir, true)?;
    let page = paths.run_dir.join("mihomo-smart.html");
    let _ = util::remove_file_if_exists(&page);
    download::download_to_file(
        &page,
        "https://github.com/vernesong/mihomo/releases/expanded_assets/Prerelease-Alpha",
        policy,
        None,
    )?;
    let html = fs::read_to_string(&page)
        .map_err(|error| BoxError::from(error).with_detail("cannot read smart release page"))?;
    let tag = parse_smart_tag(&html).ok_or_else(|| {
        BoxError::new(
            crate::error::ErrorCode::Content,
            "mihomo_smart release tag is missing",
        )
    })?;
    let url = format!(
        "https://github.com/vernesong/mihomo/releases/download/Prerelease-Alpha/mihomo-android-arm64-v8-alpha-{tag}.gz"
    );
    let mut result = install_gz_core(paths, &url, policy)?;
    result["tag"] = Value::String(tag);
    let _ = util::remove_file_if_exists(&page);
    Ok(result)
}

pub fn update_mihomo(paths: &Paths, policy: &Policy) -> Result<Value> {
    util::ensure_dir(&paths.run_dir, true)?;
    let api = paths.run_dir.join("mihomo-release.json");
    let archive = paths.run_dir.join("mihomo-download.gz");
    let temp_bin = paths.run_dir.join("mihomo-download.bin");
    let _ = util::remove_file_if_exists(&api);
    let _ = util::remove_file_if_exists(&archive);
    let _ = util::remove_file_if_exists(&temp_bin);

    let api_result = download::download_to_file(
        &api,
        "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest",
        policy,
        None,
    );
    let mut api_bytes = 0;
    let tag = match api_result {
        Ok(report) => {
            api_bytes = report.bytes;
            let text = std::fs::read_to_string(&api).map_err(|error| {
                BoxError::from(error).with_detail("cannot read mihomo release metadata")
            })?;
            let metadata: Value = serde_json::from_str(&text).map_err(|_| {
                BoxError::new(
                    crate::error::ErrorCode::Content,
                    "invalid mihomo release metadata",
                )
            })?;
            metadata
                .get("tag_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    BoxError::new(
                        crate::error::ErrorCode::Content,
                        "mihomo tag_name is missing",
                    )
                })?
        }
        Err(_) => latest_page_tag(
            paths,
            "https://github.com/MetaCubeX/mihomo/releases/latest",
            policy,
        )?,
    };
    let url = mihomo_asset_url(&tag)?;
    let archive_report = download::download_to_file(&archive, &url, policy, None)?;

    let mut child = Command::new("gzip")
        .arg("-dc")
        .arg(&archive)
        .stdout(Stdio::from(File::create(&temp_bin).map_err(|error| {
            BoxError::from(error).with_detail("cannot create mihomo temp")
        })?))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| BoxError::from(error).with_detail("cannot start gzip"))?;
    let status = child
        .wait()
        .map_err(|error| BoxError::from(error).with_detail("cannot wait for gzip"))?;
    if !status.success() {
        let _ = util::remove_file_if_exists(&temp_bin);
        return Err(BoxError::new(
            crate::error::ErrorCode::Command,
            "gzip failed to decompress mihomo",
        ));
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&temp_bin, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| BoxError::from(error).with_detail("cannot chmod mihomo"))?;
    if paths.core_bin.is_file() {
        util::ensure_dir(&paths.home.join("bin").join("backup"), true)?;
        std::fs::copy(
            &paths.core_bin,
            paths.home.join("bin").join("backup").join("mihomo.bak"),
        )
        .map_err(|error| BoxError::from(error).with_detail("cannot back up old mihomo"))?;
    }
    std::fs::rename(&temp_bin, &paths.core_bin)
        .map_err(|error| BoxError::from(error).with_detail("cannot publish mihomo"))?;
    let _ = util::remove_file_if_exists(&api);
    let _ = util::remove_file_if_exists(&archive);
    Ok(json!({
        "tag": tag,
        "path": paths.core_bin.display().to_string(),
        "archive_bytes": archive_report.bytes,
        "archive_sha256": archive_report.sha256,
        "metadata_bytes": api_bytes
    }))
}

pub fn update_yq(paths: &Paths, policy: &Policy) -> Result<Value> {
    let url = yq_url()?;
    let target = paths.home.join("bin").join("yq");
    util::ensure_dir(
        target
            .parent()
            .ok_or_else(|| BoxError::internal("yq target has no parent"))?,
        true,
    )?;
    let temp = target.with_extension("tmp");
    let _ = util::remove_file_if_exists(&temp);
    let report = match download::download_to_file(&temp, &url, policy, None) {
        Ok(report) => report,
        Err(error) => {
            let _ = util::remove_file_if_exists(&temp);
            return Err(error);
        }
    };
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| BoxError::from(error).with_detail("cannot chmod yq"))?;
    std::fs::rename(&temp, &target)
        .map_err(|error| BoxError::from(error).with_detail("cannot publish yq"))?;
    Ok(json!({
        "path": target.display().to_string(),
        "bytes": report.bytes,
        "sha256": report.sha256
    }))
}

pub fn update_xui(paths: &Paths, policy: &Policy) -> Result<Value> {
    const URL: &str =
        "https://github.com/Zephyruso/zashboard/releases/latest/download/dist-no-fonts.zip";
    let token = crate::util::now_ms();
    let archive = paths.run_dir.join(format!("xui-{token}.zip"));
    let temp = paths.run_dir.join(format!("xui-{token}-unpack"));
    let stage = paths
        .home
        .join("mihomo")
        .join(format!(".dashboard.stage.{token}"));
    let backup = paths
        .home
        .join("mihomo")
        .join(format!(".dashboard.backup.{token}"));
    let _ = fs::remove_dir_all(&temp);
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&backup);
    let result = (|| -> Result<Value> {
        util::ensure_dir(
            archive
                .parent()
                .ok_or_else(|| BoxError::internal("xui archive has no parent"))?,
            true,
        )?;
        let report = download::download_to_file(&archive, URL, policy, None)?;
        util::ensure_dir(&temp, true)?;
        let status = Command::new("unzip")
            .args([
                "-oq",
                archive.to_string_lossy().as_ref(),
                "-d",
                temp.to_string_lossy().as_ref(),
            ])
            .status()
            .map_err(|error| BoxError::from(error).with_detail("cannot run unzip"))?;
        if !status.success() {
            return Err(BoxError::new(
                crate::error::ErrorCode::Command,
                format!("unzip failed: {status}"),
            ));
        }
        let index = find_index(&temp, 0)
            .ok_or_else(|| BoxError::validation("dashboard archive has no index.html"))?;
        let html = fs::read_to_string(&index)
            .map_err(|error| BoxError::from(error).with_detail("cannot read dashboard index"))?;
        if !html.to_ascii_lowercase().contains("zashboard") {
            return Err(BoxError::validation("dashboard archive is not zashboard"));
        }
        let source = index
            .parent()
            .ok_or_else(|| BoxError::internal("dashboard index has no parent"))?;
        copy_dir(source, &stage)?;
        if !stage.join("index.html").is_file() {
            return Err(BoxError::validation("dashboard staging has no index.html"));
        }
        let target = xui_target(paths)?;
        if let Some(parent) = target.parent() {
            util::ensure_dir(parent, true)?;
        }
        let had_target = target.exists();
        if had_target {
            fs::rename(&target, &backup).map_err(|error| {
                BoxError::from(error).with_detail("cannot back up existing dashboard")
            })?;
        }
        if let Err(error) = fs::rename(&stage, &target) {
            if had_target {
                let _ = fs::rename(&backup, &target);
            }
            return Err(BoxError::from(error).with_detail("cannot publish dashboard"));
        }
        let _ = fs::remove_dir_all(&backup);
        update_external_ui(&paths.config)?;
        Ok(json!({
            "path": target.display().to_string(),
            "bytes": report.bytes,
            "sha256": report.sha256
        }))
    })();
    let _ = fs::remove_file(&archive);
    let _ = fs::remove_dir_all(&temp);
    let _ = fs::remove_dir_all(&stage);
    result
}

fn xui_target(paths: &Paths) -> Result<PathBuf> {
    let text = fs::read_to_string(&paths.config).unwrap_or_default();
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text).unwrap_or(serde_yaml::Value::Null);
    let raw = yaml
        .get("external-ui")
        .or_else(|| yaml.get("external_ui"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("./dashboard")
        .trim();
    let raw = if raw.is_empty() { "./dashboard" } else { raw };
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let relative = raw.strip_prefix("./").unwrap_or(raw);
    Ok(paths.home.join("mihomo").join(relative))
}

fn find_index(dir: &Path, depth: usize) -> Option<PathBuf> {
    if depth > 3 {
        return None;
    }
    let mut entries = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in &entries {
        let path = entry.path();
        if path.is_file() && path.file_name().is_some_and(|name| name == "index.html") {
            return Some(path);
        }
    }
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_index(&path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    util::ensure_dir(target, true)?;
    for entry in fs::read_dir(source)
        .map_err(|error| BoxError::from(error).with_detail("cannot read dashboard directory"))?
    {
        let entry = entry.map_err(BoxError::from)?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let kind = entry.file_type().map_err(BoxError::from)?;
        if kind.is_dir() {
            copy_dir(&source_path, &target_path)?;
        } else if kind.is_file() {
            fs::copy(&source_path, &target_path).map_err(BoxError::from)?;
        }
    }
    Ok(())
}

fn update_external_ui(config: &Path) -> Result<()> {
    let Ok(text) = fs::read_to_string(config) else {
        return Ok(());
    };
    let mut found = false;
    let mut output = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("external-ui:") {
            output.push_str("external-ui: ./dashboard\n");
            found = true;
        } else if trimmed.starts_with("external-ui-url:") {
            continue;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    if !found {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("external-ui: ./dashboard\n");
    }
    util::atomic_write(config, output.as_bytes(), 0o600)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn xui_target_defaults_below_mihomo_and_honors_config() {
        let root = std::env::temp_dir().join(format!("boxctl-xui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("mihomo")).expect("dirs");
        let paths = crate::paths::Paths::new(root.clone());
        assert_eq!(
            super::xui_target(&paths).expect("default"),
            root.join("mihomo/dashboard")
        );
        std::fs::write(
            &paths.config,
            "external-ui: ./webui\nexternal-ui-url: https://example.invalid\n",
        )
        .expect("config");
        assert_eq!(
            super::xui_target(&paths).expect("config"),
            root.join("mihomo/webui")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn smart_tag_is_extracted_from_release_page() {
        let html = "<a href=\"x\">smart-deadbeef</a><span>other</span>";
        assert_eq!(
            super::parse_smart_tag(html).as_deref(),
            Some("smart-deadbeef")
        );
        assert!(super::parse_smart_tag("<p>none</p>").is_none());
    }

    #[test]
    fn release_tag_page_fallback() {
        let html = r#"<a href="/MetaCubeX/mihomo/releases/tag/v1.19.0">tag</a>"#;
        assert_eq!(
            super::release_tag_from_page(html).as_deref(),
            Some("v1.19.0")
        );
        let curl = r#"<a href="/stunnel/static-curl/releases/tag/8.16.0">tag</a>"#;
        assert_eq!(
            super::release_tag_from_page(curl).as_deref(),
            Some("8.16.0")
        );
        assert!(super::release_tag_from_page("<html>none</html>").is_none());
    }

    #[test]
    fn curl_asset_url_is_pinned_and_validated() {
        let url = super::curl_url("8.16.0").expect("url");
        assert!(url.contains("/releases/download/8.16.0/curl-linux-"));
        assert!(url.ends_with("-glibc-8.16.0.tar.xz"));
        assert!(super::curl_url("8.23.0-1")
            .expect("suffix")
            .ends_with("-glibc-8.23.0-1.tar.xz"));
        assert!(super::curl_url("v8.16.0").is_err());
    }

    #[test]
    fn mihomo_asset_url_is_pinned_to_release_tag() {
        let url = super::mihomo_asset_url("v1.2.3").expect("url");
        assert!(url.contains("/releases/download/v1.2.3/"));
        assert!(
            url.ends_with("/mihomo-android-arm64-v8-v1.2.3.gz")
                || url.ends_with("/mihomo-android-amd64-v1.2.3.gz")
        );
        assert!(super::mihomo_asset_url("alpha").is_err());
    }

    #[test]
    fn yq_url_is_https_and_known_arch() {
        let url = super::yq_url().expect("arch");
        assert!(url.starts_with("https://github.com/taamarin/yq/"));
        assert!(url.ends_with("yq_android_arm64") || url.ends_with("yq_android_amd64"));
    }
}
