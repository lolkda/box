use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::download;
use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::settings::SettingsStore;
use crate::sub;
use crate::util;

const STATE_RUNNING: &str = "running";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartParams {
    pub kind: String,
    #[serde(default)]
    pub arg: Option<String>,
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogsParams {
    pub id: String,
    #[serde(default)]
    pub lines: Option<u64>,
}

pub fn start(paths: &Paths, params: StartParams) -> Result<Value> {
    if let Some(running) = running_id(paths)? {
        return Err(
            BoxError::new(ErrorCode::Busy, "a task is already running").with_detail(running)
        );
    }
    let title = match params.kind.as_str() {
        "sub-update" => "更新订阅",
        "geox" => "更新 GeoX",
        "upyq" => "更新 yq",
        "upkernel" => "更新内核",
        "upcurl" => "更新 curl",
        "upcnip" => "更新 CNIP",
        "xui" | "upxui" => "更新面板",
        _ => {
            return Err(BoxError::new(
                ErrorCode::ManagerUnavailable,
                "task kind is not implemented yet",
            )
            .with_detail(params.kind));
        }
    };
    if let Some(arg) = &params.arg {
        if arg.chars().count() > 255 || arg.chars().any(char::is_control) {
            return Err(BoxError::validation("task argument is invalid"));
        }
        if params.kind == "upkernel"
            && !arg.is_empty()
            && !matches!(arg.as_str(), "mihomo" | "mihomo_smart")
        {
            return Err(BoxError::manager_unavailable(format!(
                "unsupported core: {arg}"
            )));
        }
    }
    util::ensure_dir(&paths.tasks_dir, true)?;
    cleanup_old_tasks(paths, 20);
    let id = new_task_id(paths)?;
    let dir = paths.tasks_dir.join(&id);
    util::ensure_dir(&dir, true)?;
    write_value(&dir, "kind", &params.kind)?;
    write_value(&dir, "title", title)?;
    write_value(&dir, "state", STATE_RUNNING)?;
    let cancelable = if params.kind == "upkernel" {
        "false"
    } else {
        "true"
    };
    write_value(&dir, "cancelable", cancelable)?;
    write_value(&dir, "started_at", &now())?;
    if let Some(arg) = &params.arg {
        write_value(&dir, "arg", arg)?;
    }
    let output = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(dir.join("output.log"))
        .map_err(|error| BoxError::from(error).with_detail("cannot create task output"))?;
    let error_output = output
        .try_clone()
        .map_err(|error| BoxError::from(error).with_detail("cannot clone task output"))?;
    let exe = std::env::current_exe()
        .map_err(|error| BoxError::from(error).with_detail("cannot resolve boxctl executable"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("task-run")
        .arg(&id)
        .arg("--root")
        .arg(&paths.home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(error_output));
    // Put task runner in its own process group so cancel can terminate child tools cleanly.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            let _ = libc::setpgid(0, 0);
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|error| BoxError::from(error).with_detail("cannot spawn task runner"))?;
    write_value(&dir, "pid", &child.id().to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    if params.wait {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            let current = info(&dir)?;
            if current["state"].as_str() != Some(STATE_RUNNING) {
                return Ok(current);
            }
            if std::time::Instant::now() >= deadline {
                return Err(BoxError::new(
                    ErrorCode::Interrupted,
                    "task did not finish within the wait window",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    info(&dir)
}

pub fn run_task(paths: &Paths, id: &str) -> i32 {
    let dir = paths.tasks_dir.join(id);
    if !valid_id(id) || !dir.is_dir() {
        eprintln!("task does not exist: {id}");
        return 1;
    }
    let kind = read_value(&dir, "kind");
    let arg = read_value(&dir, "arg");
    let result = (|| -> Result<Value> {
        let store = SettingsStore::open(paths.clone())?;
        match kind.as_str() {
            "upcnip" => update_cnip(paths, &store),
            "xui" | "upxui" => {
                let policy = download::policy_from_settings(&store)?;
                crate::tools::update_xui(paths, &policy)
            }
            "geox" => {
                let policy = download::policy_from_settings(&store)?;
                crate::geo::update(paths, &policy)
            }
            "upyq" => {
                let policy = download::policy_from_settings(&store)?;
                crate::tools::update_yq(paths, &policy)
            }
            "upcurl" => {
                let policy = download::policy_from_settings(&store)?;
                crate::tools::update_curl(paths, &policy)
            }
            "upkernel" => {
                let policy = download::policy_from_settings(&store)?;
                let core = if arg.is_empty() {
                    "mihomo".to_string()
                } else {
                    arg
                };
                match core.as_str() {
                    "mihomo" => crate::tools::update_mihomo(paths, &policy),
                    "mihomo_smart" => crate::tools::update_mihomo_smart(paths, &policy),
                    _ => Err(BoxError::manager_unavailable(format!(
                        "unsupported core: {core}"
                    ))),
                }
            }
            "sub-update" => {
                let policy = download::policy_from_settings(&store)?;
                let settings = store.load()?;
                let keep_last_good = settings
                    .get("subscription_keep_last_good")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == "true");
                sub::update(
                    paths,
                    sub::UpdateParams {
                        name: Some(arg).filter(|value| !value.is_empty()),
                    },
                    &policy,
                    keep_last_good,
                )
            }
            other => Err(BoxError::new(
                ErrorCode::ManagerUnavailable,
                format!("task kind is not implemented: {other}"),
            )),
        }
    })();
    match result {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
            let _ = write_value(&dir, "exit_code", "0");
            let _ = write_value(&dir, "state", "success");
            let _ = write_value(&dir, "finished_at", &now());
            0
        }
        Err(error) => {
            eprintln!("{error}");
            let _ = write_value(&dir, "exit_code", "1");
            let _ = write_value(&dir, "state", "failed");
            let _ = write_value(&dir, "finished_at", &now());
            1
        }
    }
}

pub fn list(paths: &Paths) -> Result<Value> {
    let mut items = Vec::new();
    if paths.tasks_dir.is_dir() {
        for entry in fs::read_dir(&paths.tasks_dir)
            .map_err(|error| BoxError::from(error).with_detail("cannot read task directory"))?
        {
            let entry = entry.map_err(BoxError::from)?;
            if !entry.path().is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !valid_id(&id) {
                continue;
            }
            update_interrupted(&entry.path());
            items.push((id, info(&entry.path())?));
        }
    }
    items.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(json!({"items": items.into_iter().map(|(_, item)| item).collect::<Vec<_>>() }))
}

fn running_id(paths: &Paths) -> Result<Option<String>> {
    if !paths.tasks_dir.is_dir() {
        return Ok(None);
    }
    for entry in fs::read_dir(&paths.tasks_dir)
        .map_err(|error| BoxError::from(error).with_detail("cannot read task directory"))?
    {
        let entry = entry.map_err(BoxError::from)?;
        if !entry.path().is_dir() {
            continue;
        }
        update_interrupted(&entry.path());
        if read_value(&entry.path(), "state") == STATE_RUNNING {
            return Ok(entry.file_name().to_str().map(str::to_string));
        }
    }
    Ok(None)
}

fn update_cnip(paths: &Paths, store: &SettingsStore) -> Result<Value> {
    let settings = store.load()?;
    let enabled = |key: &str| {
        settings
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value == "true")
    };
    let bypass = enabled("bypass_cn_ip");
    let v4_enabled = bypass && enabled("bypass_cn_ip_v4");
    let v6_enabled = bypass && enabled("ipv6") && enabled("bypass_cn_ip_v6");
    if !v4_enabled && !v6_enabled {
        return Ok(json!({"updated": [], "reloaded": [], "reason": "disabled"}));
    }
    let policy = download::policy_from_settings(store)?;
    let mut updated = Vec::new();
    let mut reloaded = Vec::new();
    if v4_enabled {
        let url = string_setting(&settings, "cn_ip_url")?;
        util::ensure_dir(
            paths
                .cnip_v4_file
                .parent()
                .ok_or_else(|| BoxError::internal("CNIP v4 path has no parent"))?,
            true,
        )?;
        let report = download::download_to_file(&paths.cnip_v4_file, &url, &policy, None)?;
        updated.push(json!({
            "family": "ipv4",
            "path": paths.cnip_v4_file.display().to_string(),
            "bytes": report.bytes,
            "sha256": report.sha256
        }));
    }
    if v6_enabled {
        let url = string_setting(&settings, "cn_ipv6_url")?;
        util::ensure_dir(
            paths
                .cnip_v6_file
                .parent()
                .ok_or_else(|| BoxError::internal("CNIP v6 path has no parent"))?,
            true,
        )?;
        let report = download::download_to_file(&paths.cnip_v6_file, &url, &policy, None)?;
        updated.push(json!({
            "family": "ipv6",
            "path": paths.cnip_v6_file.display().to_string(),
            "bytes": report.bytes,
            "sha256": report.sha256
        }));
    }
    let core_running = crate::service::status(paths)
        .ok()
        .and_then(|s| s["running"].as_bool())
        .unwrap_or(false);
    let ipset_available = crate::network::detect(paths).ipset;
    if core_running && ipset_available {
        if v4_enabled {
            reload_cnip_set(&paths.cnip_v4_file, "cnip", "inet")?;
            reloaded.push("cnip");
        }
        if v6_enabled {
            reload_cnip_set(&paths.cnip_v6_file, "cnip6", "inet6")?;
            reloaded.push("cnip6");
        }
    }
    Ok(json!({"updated": updated, "reloaded": reloaded}))
}

fn string_setting(settings: &serde_json::Map<String, Value>, key: &str) -> Result<String> {
    settings
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| BoxError::validation(format!("{key} is not configured")))
}

pub fn reload_cnip_set(path: &Path, set: &str, family: &str) -> Result<()> {
    let text = fs::read_to_string(path).map_err(|error| {
        BoxError::from(error).with_detail(format!("cannot read {}", path.display()))
    })?;
    let entries: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if entries.is_empty() {
        return Err(BoxError::validation(format!(
            "CNIP list is empty: {}",
            path.display()
        )));
    }
    let mut input = format!(
        "create {set} hash:net family {family} hashsize 8192 maxelem 65536 -exist\nflush {set}\n"
    );
    for entry in entries {
        input.push_str(&format!("add {set} {entry}\n"));
    }
    let mut child = Command::new("ipset")
        .args(["restore", "-exist"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| BoxError::from(error).with_detail("cannot start ipset restore"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| BoxError::internal("ipset stdin is unavailable"))?;
        stdin.write_all(input.as_bytes()).map_err(|error| {
            BoxError::from(error).with_detail("cannot write ipset restore input")
        })?;
    }
    let status = child
        .wait()
        .map_err(|error| BoxError::from(error).with_detail("cannot wait for ipset"))?;
    if !status.success() {
        return Err(BoxError::new(
            ErrorCode::Command,
            format!("ipset restore failed for {set}"),
        )
        .with_detail(format!("status {status}")));
    }
    Ok(())
}

fn new_task_id(paths: &Paths) -> Result<String> {
    let base = format!("{}-{}", util::now_ms() / 1000, std::process::id());
    let mut candidate = base.clone();
    let mut index = 2;
    while paths.tasks_dir.join(&candidate).exists() {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    Ok(candidate)
}

pub fn running_count(paths: &Paths) -> Result<usize> {
    if !paths.tasks_dir.is_dir() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(&paths.tasks_dir)
        .map_err(|error| BoxError::from(error).with_detail("cannot read task directory"))?
    {
        let entry = entry.map_err(BoxError::from)?;
        if !entry.path().is_dir() {
            continue;
        }
        update_interrupted(&entry.path());
        if read_value(&entry.path(), "state") == STATE_RUNNING {
            count += 1;
        }
    }
    Ok(count)
}

pub fn status(paths: &Paths, params: IdParams) -> Result<Value> {
    let dir = task_dir(paths, &params.id)?;
    if !dir.is_dir() {
        return Err(not_found(&params.id));
    }
    update_interrupted(&dir);
    info(&dir)
}

pub fn logs(paths: &Paths, params: LogsParams) -> Result<Value> {
    let count = params.lines.unwrap_or(200);
    if count > 10_000 {
        return Err(BoxError::validation("task logs lines must be 0..=10000"));
    }
    let dir = task_dir(paths, &params.id)?;
    if !dir.is_dir() {
        return Err(not_found(&params.id));
    }
    let file = dir.join("output.log");
    let text = util::read_to_string_optional(&file)?.unwrap_or_default();
    let mut lines = text
        .lines()
        .rev()
        .take(count as usize)
        .map(str::to_string)
        .collect::<Vec<_>>();
    lines.reverse();
    Ok(json!({
        "id": params.id,
        "file": file.display().to_string(),
        "lines": lines
    }))
}

pub fn interrupt_all(paths: &Paths) -> Result<usize> {
    let items = list(paths)?;
    let Some(items) = items.get("items").and_then(Value::as_array) else {
        return Ok(0);
    };
    let mut count = 0;
    for item in items {
        if item.get("state").and_then(Value::as_str) != Some(STATE_RUNNING) {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let dir = paths.tasks_dir.join(id);
        if read_value(&dir, "cancelable") == "true" {
            let _ = cancel(paths, IdParams { id: id.to_string() });
        } else {
            let _ = write_value(&dir, "state", "interrupted");
            let _ = write_value(&dir, "finished_at", &now());
        }
        count += 1;
    }
    Ok(count)
}

pub fn cancel(paths: &Paths, params: IdParams) -> Result<Value> {
    let dir = task_dir(paths, &params.id)?;
    if !dir.is_dir() {
        return Err(not_found(&params.id));
    }
    update_interrupted(&dir);
    if read_value(&dir, "state") != STATE_RUNNING {
        return Err(BoxError::validation("task has already ended"));
    }
    if read_value(&dir, "cancelable") != "true" {
        return Err(BoxError::validation("task is not cancelable"));
    }
    let Some(pid) = read_pid(&dir) else {
        return Err(BoxError::new(
            ErrorCode::Interrupted,
            "task process does not exist",
        ));
    };
    write_value(&dir, "cancel_requested", "canceled")?;
    let result = unsafe {
        // Kill the entire task process group first, falling back to single PID.
        let pg = libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
        if pg == 0 {
            0
        } else {
            libc::kill(pid as libc::pid_t, libc::SIGTERM)
        }
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            mark_interrupted(&dir);
            return info(&dir);
        }
        return Err(BoxError::from(error).with_detail("cannot signal task process"));
    }
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(50));
        if !pid_alive(pid) {
            write_value(&dir, "state", "canceled")?;
            write_value(&dir, "finished_at", &now())?;
            return info(&dir);
        }
    }
    // Force SIGKILL after 1s grace period so cancel never leaves a stuck running task.
    unsafe {
        let _ = libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        let _ = libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    thread::sleep(Duration::from_millis(100));
    write_value(&dir, "state", "canceled")?;
    write_value(&dir, "finished_at", &now())?;
    info(&dir)
}

fn cleanup_old_tasks(paths: &Paths, keep: usize) {
    let Ok(entries) = fs::read_dir(&paths.tasks_dir) else {
        return;
    };
    let mut dirs = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| read_value(&e.path(), "state") != STATE_RUNNING)
        .map(|e| {
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (mtime, e.path())
        })
        .collect::<Vec<_>>();
    if dirs.len() <= keep {
        return;
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in dirs.into_iter().skip(keep) {
        let _ = fs::remove_dir_all(path);
    }
}

fn task_dir(paths: &Paths, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        return Err(BoxError::usage("invalid task id"));
    }
    Ok(paths.tasks_dir.join(id))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-')
}

fn not_found(id: &str) -> BoxError {
    BoxError::new(ErrorCode::NotFound, format!("task does not exist: {id}"))
}

fn info(dir: &Path) -> Result<Value> {
    let id = dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let state = {
        let value = read_value(dir, "state");
        if value.is_empty() {
            "unknown".to_string()
        } else {
            value
        }
    };
    let pid = read_pid(dir);
    let started = read_value(dir, "started_at");
    let finished = read_value(dir, "finished_at");
    let exit_code = read_value(dir, "exit_code").parse::<i32>().ok();
    let cancelable = read_value(dir, "cancelable") == "true";
    let output_size = fs::metadata(dir.join("output.log")).map_or(0, |metadata| metadata.len());
    Ok(json!({
        "id": id,
        "kind": read_value(dir, "kind"),
        "title": read_value(dir, "title"),
        "state": state,
        "pid": pid,
        "started_at": started,
        "finished_at": finished,
        "exit_code": exit_code,
        "cancelable": cancelable,
        "output_size": output_size
    }))
}

fn update_interrupted(dir: &Path) {
    if read_value(dir, "state") != STATE_RUNNING {
        return;
    }
    match read_pid(dir) {
        Some(pid) if pid_alive(pid) => {}
        _ => mark_interrupted(dir),
    }
}

fn mark_interrupted(dir: &Path) {
    let _ = write_value(dir, "state", "interrupted");
    let _ = write_value(dir, "finished_at", &now());
}

fn read_pid(dir: &Path) -> Option<u32> {
    read_value(dir, "pid").parse::<u32>().ok()
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let result = unsafe {
        // SAFETY: signal 0 only probes whether the PID exists; no memory is
        // dereferenced and the PID comes from a validated numeric file.
        libc::kill(pid as libc::pid_t, 0)
    };
    if result != 0 {
        return false;
    }
    // A killed child can remain a zombie until its parent reaps it; kill(0)
    // still succeeds for zombies, so check /proc state before declaring the
    // task alive.
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    let Some(close) = stat.rfind(')') else {
        return true;
    };
    !stat[close + 1..].trim_start().starts_with('Z')
}

fn read_value(dir: &Path, name: &str) -> String {
    util::read_to_string_optional(&dir.join(name))
        .ok()
        .flatten()
        .map(|value| value.trim_end_matches('\n').to_string())
        .unwrap_or_default()
}

fn write_value(dir: &Path, name: &str, value: &str) -> Result<()> {
    let mut text = value.to_string();
    text.push('\n');
    util::atomic_write(&dir.join(name), text.as_bytes(), 0o600)
}

fn now() -> String {
    util::format_unix_utc(util::now_ms() / 1000)
}

#[cfg(test)]
mod tests {
    use super::{cancel, list, logs, status, IdParams, LogsParams};
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-task-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("run/tasks/20260101120000-1")).expect("task dir");
        let dir = root.join("run/tasks/20260101120000-1");
        fs::write(dir.join("kind"), "sub-update\n").expect("kind");
        fs::write(dir.join("title"), "update\n").expect("title");
        fs::write(dir.join("state"), "running\n").expect("state");
        fs::write(dir.join("cancelable"), "true\n").expect("cancelable");
        fs::write(dir.join("started_at"), "2026-01-01 12:00:00\n").expect("started");
        fs::write(dir.join("output.log"), "a\nb\nc\n").expect("output");
        root
    }

    #[test]
    fn lists_and_reads_task_logs() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let data = list(&paths).expect("list");
        assert_eq!(data["items"].as_array().expect("items").len(), 1);
        let logs = logs(
            &paths,
            LogsParams {
                id: "20260101120000-1".to_string(),
                lines: Some(2),
            },
        )
        .expect("logs");
        assert_eq!(logs["lines"], serde_json::json!(["b", "c"]));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancel_stops_a_task_process() {
        let root = temp_root();
        let dir = root.join("run/tasks/20260101120000-2");
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("state"), "running\n").expect("state");
        fs::write(dir.join("cancelable"), "true\n").expect("cancelable");
        let mut child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        fs::write(dir.join("pid"), format!("{}\n", child.id())).expect("pid");
        let paths = Paths::new(root.clone());
        let result = cancel(
            &paths,
            IdParams {
                id: "20260101120000-2".to_string(),
            },
        )
        .expect("cancel");
        assert_eq!(result["state"], "canceled");
        let _ = child.wait();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_marks_dead_pid_interrupted() {
        let root = temp_root();
        let dir = root.join("run/tasks/20260101120000-1");
        fs::write(dir.join("pid"), "999999\n").expect("pid");
        let paths = Paths::new(root.clone());
        let data = status(
            &paths,
            IdParams {
                id: "20260101120000-1".to_string(),
            },
        )
        .expect("status");
        assert_eq!(data["state"], "interrupted");
        let _ = fs::remove_dir_all(root);
    }
}
