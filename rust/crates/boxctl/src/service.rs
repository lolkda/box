use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{BoxError, ErrorCode, Result};
use crate::network;
use crate::network_rules;
use crate::paths::Paths;
use crate::settings::SettingsStore;
use crate::upgrade;
use crate::util;

const DEFAULT_HEALTH_WINDOW: Duration = Duration::from_secs(5);
const STOP_WINDOW: Duration = Duration::from_secs(2);
const NET_ADMIN_GID: u32 = 3005;
const MAX_RETAINED_CANDIDATES: usize = 20;
const CORE_NOFILE_LIMIT: u64 = 1_000_000;

#[derive(Debug, Clone)]
struct Tx {
    txid: String,
    dir: std::path::PathBuf,
    candidate: std::path::PathBuf,
}

pub fn status(paths: &Paths) -> Result<Value> {
    let pid = read_pid(paths)?;
    let running = pid.is_some_and(|pid| core_alive(paths, pid));
    let state = read_service_state(paths).unwrap_or_else(|| json!({}));
    Ok(json!({
        "running": running,
        "pid": pid.filter(|pid| core_alive(paths, *pid)),
        "txid": state.get("txid").cloned().unwrap_or(Value::Null),
        "phase": state.get("phase").cloned().unwrap_or_else(|| json!("idle")),
        "last_error": state.get("last_error").cloned().unwrap_or(Value::Null),
        "manual": state.get("phase").and_then(Value::as_str) == Some("manual")
    }))
}

pub fn start(paths: &Paths) -> Result<Value> {
    let mut runner = network::SystemRunner;
    start_with_window(paths, DEFAULT_HEALTH_WINDOW, &mut runner, None)
}

fn start_with_window(
    paths: &Paths,
    window: Duration,
    runner: &mut dyn network::Runner,
    fault: Option<&str>,
) -> Result<Value> {
    if status(paths)?["running"].as_bool().unwrap_or(false) {
        return Err(BoxError::new(
            ErrorCode::Conflict,
            "core is already running",
        ));
    }
    let upgrade = upgrade::status(paths)?;
    if upgrade["required"].as_bool().unwrap_or(false)
        && !upgrade["confirmed"].as_bool().unwrap_or(false)
    {
        return Err(BoxError::new(
            ErrorCode::NeedsConfirmation,
            "upgrade confirmation is required before starting the core",
        ));
    }
    if !paths.core_bin.is_file() {
        return Err(
            BoxError::new(ErrorCode::NotFound, "core binary does not exist")
                .with_detail(paths.core_bin.display().to_string()),
        );
    }
    if !paths.config.is_file() {
        return Err(
            BoxError::new(ErrorCode::NotFound, "core config does not exist")
                .with_detail(paths.config.display().to_string()),
        );
    }
    let tx = begin_tx(paths, "start")?;
    set_phase(paths, &tx, "preparing", None)?;
    let mode = selected_mode(paths);
    if !matches!(mode.as_deref(), Some("tun") | Some("mixed")) {
        disable_tun_candidate(paths, &tx.candidate)?;
    } else if mode.as_deref() == Some("mixed") {
        enable_tun_candidate(paths, &tx.candidate)?;
    }
    prepare_candidate(paths, &tx.candidate, mode.as_deref())?;
    if !check_candidate(paths, &tx.candidate) {
        fail_manual(paths, &tx, "core config validation failed")?;
        return Err(BoxError::new(
            ErrorCode::Command,
            "core config validation failed",
        ));
    }
    // Reap any orphan core process not tracked by box.pid before binding ports.
    kill_orphan_cores(paths, None);
    set_phase(paths, &tx, "switching", None)?;
    if fault == Some("switching") {
        return Err(BoxError::internal("injected fault after switching"));
    }
    let mut child = spawn_core(paths, &tx.candidate).inspect_err(|error| {
        let _ = fail_manual(paths, &tx, &error.message);
    })?;
    let pid = child.id();
    write_pid(paths, pid)?;
    let cgroup_warnings = apply_cgroups_for_pid(paths, pid);
    if fault == Some("spawned") {
        let _ = child.kill();
        let _ = child.wait();
        let _ = util::remove_file_if_exists(&paths.box_pid);
        return Err(BoxError::internal("injected fault after spawn"));
    }
    let started = Instant::now();
    while started.elapsed() < window {
        if let Some(exit) = child.try_wait().ok().flatten() {
            let _ = util::remove_file_if_exists(&paths.box_pid);
            let _ = fail_manual(paths, &tx, &format!("core exited during startup: {exit}"));
            return Err(BoxError::new(
                ErrorCode::Command,
                "core exited during startup",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
    set_phase(paths, &tx, "verifying", None)?;
    if fault == Some("verifying") {
        let _ = child.kill();
        let _ = child.wait();
        let _ = util::remove_file_if_exists(&paths.box_pid);
        return Err(BoxError::internal("injected fault after verifying"));
    }
    if !core_alive(paths, pid) {
        let _ = child.kill();
        let _ = child.wait();
        let _ = util::remove_file_if_exists(&paths.box_pid);
        let _ = fail_manual(paths, &tx, "core is not alive after startup window");
        return Err(BoxError::new(
            ErrorCode::Command,
            "core is not alive after startup window",
        ));
    }
    let plan = load_network_plan(paths)?;
    if let Some(plan) = &plan {
        if plan.rules.is_empty()
            && !matches!(plan.requested_mode.as_str(), "tun" | "mixed")
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = util::remove_file_if_exists(&paths.box_pid);
            let _ = fail_manual(paths, &tx, "required network rules are unavailable");
            return Err(BoxError::new(
                ErrorCode::ManagerUnavailable,
                "required network rules are unavailable",
            )
            .with_detail(plan.warnings.join("; ")));
        }
        if matches!(plan.requested_mode.as_str(), "tun" | "mixed") {
            let tun_dev = tun_device_name(paths);
            let _ = wait_tun_device(&tun_dev, Duration::from_secs(6));
        }
        // Always ensure CNIP ipsets are loaded when ipset is available and enabled.
        let cnip_warnings = load_cnip_ipsets_if_needed(paths);
        let _ = cnip_warnings;
        if !plan.rules.is_empty() {
            // Remove leftovers from any previous mode before adding the new plan.
            let _ = cleanup_all_network(paths, runner);
            if let Err(error) = network::apply(plan, runner) {
                let _ = set_phase(paths, &tx, "rolling_back", Some(&error.message));
                if fault == Some("rolling_back") {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = util::remove_file_if_exists(&paths.box_pid);
                    return Err(BoxError::internal("injected fault during rollback"));
                }
                let rollback = network::cleanup(plan, runner);
                let _ = child.kill();
                let _ = child.wait();
                let _ = util::remove_file_if_exists(&paths.box_pid);
                let original = error
                    .detail
                    .clone()
                    .unwrap_or_else(|| error.message.clone());
                let detail = match rollback {
                    Ok(()) => original,
                    Err(cleanup) => {
                        let cleanup_detail = cleanup
                            .detail
                            .clone()
                            .unwrap_or_else(|| cleanup.message.clone());
                        format!("{original}; rollback failed: {cleanup_detail}")
                    }
                };
                let _ = fail_manual(paths, &tx, &detail);
                return Err(error.with_detail(detail));
            }
        }
        // Apply sysctl / IPv6 disable / dynamic LOCAL_IP_V4/6 after rules are active.
        apply_runtime_network_post_start(paths, plan, runner);
    }
    if let Err(error) = health_check(&tx.candidate) {
        let _ = set_phase(paths, &tx, "rolling_back", Some(&error.message));
        if let Some(plan) = &plan {
            if !plan.rules.is_empty() {
                let _ = network::cleanup(plan, runner);
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = util::remove_file_if_exists(&paths.box_pid);
        let _ = fail_manual(paths, &tx, &error.message);
        return Err(error);
    }
    set_phase(paths, &tx, "applied", None)?;
    write_last_good(paths, &tx)?;
    cleanup_old_candidates(paths, MAX_RETAINED_CANDIDATES);
    set_intent(paths, "running")?;
    let _ = write_control(paths, "manual", "running")?;
    let _ = util::remove_file_if_exists(&paths.event_stop);
    let mut warnings = plan
        .as_ref()
        .map(|plan| {
            plan.warnings
                .iter()
                .map(|message| json!({"code": "E_NETWORK_DEGRADED", "message": message}))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![json!({"code": "E_NETWORK_PLAN_UNAVAILABLE", "message": "settings are unavailable; network rules were not applied"})]
        });
    for message in cgroup_warnings {
        warnings.push(json!({"code": "E_CGROUP_DEGRADED", "message": message}));
    }
    Ok(json!({
        "action": "start",
        "status": status(paths)?,
        "rules_applied": plan.as_ref().is_some_and(|plan| !plan.rules.is_empty() || plan.requested_mode == "tun"),
        "warnings": warnings
    }))
}

pub fn start_for_restart(paths: &Paths) -> Result<Value> {
    let previous = control(paths);
    let result = start(paths);
    if result.is_ok() {
        let mode = previous["mode"].as_str().unwrap_or("auto");
        let _ = write_control(paths, mode, "running");
    }
    result
}

pub fn stop(paths: &Paths) -> Result<Value> {
    let mut runner = network::SystemRunner;
    stop_internal_with_runner(paths, true, &mut runner)
}

fn stop_internal(paths: &Paths, interrupt_tasks: bool) -> Result<Value> {
    let mut runner = network::SystemRunner;
    stop_internal_with_runner(paths, interrupt_tasks, &mut runner)
}

fn stop_internal_with_runner(
    paths: &Paths,
    interrupt_tasks: bool,
    runner: &mut dyn network::Runner,
) -> Result<Value> {
    let tx = begin_tx(paths, "stop")?;
    set_phase(paths, &tx, "switching", None)?;
    // Clean up network rules BEFORE stopping the core (module-service-stop-cleanup R-002).
    // If cleanup fails, abort stop and leave core running so transparent proxy does not blackhole traffic.
    let rules_cleaned = match cleanup_all_network(paths, runner) {
        Ok(cleaned) => cleaned,
        Err(error) => {
            let _ = fail_manual(
                paths,
                &tx,
                error.detail.as_deref().unwrap_or(&error.message),
            );
            return Err(error);
        }
    };
    // Restore IPv6 sysctl / unreachable rule if previously disabled.
    network::restore_ipv6_policy(runner);
    let _ = util::remove_file_if_exists(&paths.event_stop);
    set_intent(paths, "stopped")?;
    let _ = write_control(paths, "manual", "stopped")?;
    if interrupt_tasks {
        let _ = crate::task::interrupt_all(paths);
    }
    let pid = read_pid(paths)?;
    if let Some(pid) = pid {
        if core_alive(paths, pid) {
            terminate(pid, libc::SIGTERM);
            let started = Instant::now();
            while started.elapsed() < STOP_WINDOW && core_alive(paths, pid) {
                thread::sleep(Duration::from_millis(100));
            }
            if core_alive(paths, pid) {
                terminate(pid, libc::SIGKILL);
                thread::sleep(Duration::from_millis(100));
            }
            if core_alive(paths, pid) {
                fail_manual(paths, &tx, "core did not stop")?;
                return Err(BoxError::new(ErrorCode::Command, "core did not stop"));
            }
        }
    }
    if let Some(pid) = pid {
        reap_child(pid);
    }
    // Also clean up any orphan core processes not tracked by box.pid.
    kill_orphan_cores(paths, pid);
    let _ = util::remove_file_if_exists(&paths.box_pid);
    set_phase(paths, &tx, "applied", None)?;
    Ok(json!({
        "action": "stop",
        "status": status(paths)?,
        "rules_cleaned": rules_cleaned,
        "warnings": []
    }))
}

pub fn restart(paths: &Paths) -> Result<Value> {
    // A restart can be requested by a running sub-update task. Do not cancel
    // that task just to reload the core; explicit stop still interrupts tasks.
    stop_internal(paths, false)?;
    start(paths)
}

fn reload_url(endpoint: &str) -> Result<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(BoxError::new(
            ErrorCode::Reload,
            "external-controller is not configured",
        ));
    }
    let (scheme, authority) = if let Some(rest) = endpoint.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        ("http", rest)
    } else {
        ("http", endpoint)
    };
    if authority.is_empty() || authority.contains('/') {
        return Err(BoxError::new(
            ErrorCode::Reload,
            "external-controller authority is invalid",
        ));
    }
    Ok(format!("{scheme}://{authority}/configs?force=true"))
}

pub fn reload(paths: &Paths) -> Result<Value> {
    // Stage and prepare a fresh candidate config so Mihomo reloads the latest
    // config.yaml + settings without modifying the user's source config.yaml.
    let candidate_path = stage_reload_candidate(paths)?;
    let config_to_read = candidate_path.as_deref().unwrap_or(&paths.config);
    let config = fs::read_to_string(config_to_read)
        .map_err(|error| BoxError::from(error).with_detail("cannot read core config"))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&config).map_err(|error| {
        BoxError::new(ErrorCode::Reload, "core config is not valid YAML")
            .with_detail(error.to_string())
    })?;
    let endpoint = value
        .get("external-controller")
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BoxError::new(ErrorCode::Reload, "external-controller is not configured"))?;
    let secret = value
        .get("secret")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default();
    let url = reload_url(endpoint)?;
    let roots = crate::download::system_root_certs()
        .map(ureq::tls::RootCerts::from)
        .unwrap_or(ureq::tls::RootCerts::WebPki);
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build())
        .build()
        .new_agent();
    let mut request = agent.put(&url);
    if !secret.is_empty() {
        request = request.header("Authorization", &format!("Bearer {secret}"));
    }
    let payload = json!({
        "path": "",
        "payload": if candidate_path.is_some() { config.as_str() } else { "" }
    })
    .to_string();
    let response = request
        .header("Content-Type", "application/json")
        .send(&payload)
        .map_err(|error| {
            BoxError::new(ErrorCode::Reload, "core reload request failed")
                .with_detail(error.to_string())
        })?;
    if !response.status().is_success() {
        return Err(BoxError::new(
            ErrorCode::Reload,
            format!("core reload returned HTTP {}", response.status().as_u16()),
        ));
    }
    cleanup_old_candidates(paths, MAX_RETAINED_CANDIDATES);
    Ok(json!({
        "action": "reload",
        "status": response.status().as_u16(),
        "candidate": candidate_path.map(|p| p.display().to_string())
    }))
}

pub fn reload_if_running(paths: &Paths) -> Result<bool> {
    if !status(paths)?["running"].as_bool().unwrap_or(false) {
        return Ok(false);
    }
    reload(paths)?;
    Ok(true)
}

fn stage_reload_candidate(paths: &Paths) -> Result<Option<std::path::PathBuf>> {
    if !paths.config.is_file() {
        return Ok(None);
    }
    if !paths.settings.is_file() {
        return Ok(None);
    }
    let txid = format!("reload-{}-{}", util::now_ms(), std::process::id());
    let candidate_dir = paths.candidates_dir.join(&txid);
    util::ensure_dir(&candidate_dir, true)?;
    let candidate = candidate_dir.join("config.yaml");
    fs::copy(&paths.config, &candidate)
        .map_err(|error| BoxError::from(error).with_detail("cannot stage reload candidate"))?;
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o600))
        .map_err(|error| BoxError::from(error).with_detail("cannot chmod reload candidate"))?;
    let mode = selected_mode(paths);
    if !matches!(mode.as_deref(), Some("tun") | Some("mixed")) {
        disable_tun_candidate(paths, &candidate)?;
    } else if mode.as_deref() == Some("mixed") {
        enable_tun_candidate(paths, &candidate)?;
    }
    prepare_candidate(paths, &candidate, mode.as_deref())?;
    if paths.core_bin.is_file() && !check_candidate(paths, &candidate) {
        let _ = fs::remove_dir_all(&candidate_dir);
        return Err(BoxError::new(
            ErrorCode::Reload,
            "core config validation failed before reload",
        ));
    }
    Ok(Some(candidate))
}

pub fn check_config(paths: &Paths) -> Result<Value> {
    if !paths.core_bin.is_file() {
        return Err(
            BoxError::new(ErrorCode::NotFound, "core binary does not exist")
                .with_detail(paths.core_bin.display().to_string()),
        );
    }
    if !paths.config.is_file() {
        return Err(
            BoxError::new(ErrorCode::NotFound, "core config does not exist")
                .with_detail(paths.config.display().to_string()),
        );
    }
    let output = Command::new(&paths.core_bin)
        .arg("-t")
        .arg("-d")
        .arg(paths.home.join("mihomo"))
        .arg("-f")
        .arg(&paths.config)
        .output()
        .map_err(|error| BoxError::from(error).with_detail("cannot run mihomo -t"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(BoxError::new(
            ErrorCode::Validation,
            "mihomo configuration check failed",
        )
        .with_detail(format!("{stdout}{stderr}").trim().to_string()));
    }
    Ok(json!({
        "valid": true,
        "config": paths.config.display().to_string(),
        "output": format!("{stdout}{stderr}").trim()
    }))
}

fn health_check(config: &Path) -> Result<()> {
    let text = fs::read_to_string(config)
        .map_err(|error| BoxError::from(error).with_detail("cannot read candidate config"))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        BoxError::new(ErrorCode::Command, "candidate config is not valid YAML")
            .with_detail(error.to_string())
    })?;
    if value
        .get("tun")
        .and_then(|tun| tun.get("enable"))
        .and_then(serde_yaml::Value::as_bool)
        .unwrap_or(false)
        && !Path::new("/dev/net/tun").exists()
    {
        return Err(BoxError::new(
            ErrorCode::Command,
            "health check failed: TUN device is unavailable",
        ));
    }
    let mut ports = Vec::new();
    for key in [
        "port",
        "socks-port",
        "mixed-port",
        "redir-port",
        "tproxy-port",
    ] {
        if let Some(port) = value.get(key).and_then(serde_yaml::Value::as_u64) {
            if let Ok(port) = u16::try_from(port) {
                ports.push(port);
            }
        }
    }
    if let Some(endpoint) = value
        .get("external-controller")
        .and_then(serde_yaml::Value::as_str)
    {
        if let Some(port) = endpoint
            .rsplit(':')
            .next()
            .and_then(|value| value.parse::<u16>().ok())
        {
            ports.push(port);
        }
    }
    for port in ports {
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        std::net::TcpStream::connect_timeout(&address, Duration::from_millis(250)).map_err(
            |_| {
                BoxError::new(
                    ErrorCode::Command,
                    format!("health check failed: port {port} is not listening"),
                )
            },
        )?;
    }
    Ok(())
}

fn cleanup_all_network(paths: &Paths, runner: &mut dyn network::Runner) -> Result<bool> {
    if !paths.settings.is_file() {
        return Ok(false);
    }
    let store = SettingsStore::open(paths.clone())?;
    let mut settings = Value::Object(store.load()?);
    enrich_network_settings(paths, &mut settings);
    let caps = network::detect(paths);
    let rules = network_rules::load_for_plan(paths).ok();
    let cleaned = network::cleanup_all(
        &settings,
        caps,
        dns_listen_port(paths),
        rules.as_ref(),
        runner,
    )?;
    Ok(network::cleanup_stale(runner) || cleaned)
}

pub fn load_network_plan(paths: &Paths) -> Result<Option<network::Plan>> {
    if !paths.settings.is_file() {
        return Ok(None);
    }
    let store = SettingsStore::open(paths.clone())?;
    let mut settings = Value::Object(store.load()?);
    enrich_network_settings(paths, &mut settings);
    let rules = network_rules::load_for_plan(paths)?;
    build_network_plan(paths, &settings, &rules)
}

pub fn enrich_network_settings(paths: &Paths, settings: &mut Value) {
    settings["__proxy_uids"] = Value::Array(
        proxy_uids(paths, settings)
            .into_iter()
            .map(|uid| Value::Number(uid.into()))
            .collect(),
    );
    settings["__proxy_gids"] = Value::Array(
        proxy_gids(paths)
            .into_iter()
            .map(|gid| Value::Number(gid.into()))
            .collect(),
    );
    let (allow_aps, ignore_aps) = read_ap_lists(paths);
    settings["__ap_allow"] = Value::Array(allow_aps.into_iter().map(Value::String).collect());
    settings["__ap_ignore"] = Value::Array(ignore_aps.into_iter().map(Value::String).collect());
    settings["__tun_device"] = Value::String(tun_device_name(paths));
    let (fake_ip_v4, fake_ip_v6) = fake_ip_ranges(paths);
    if let Some(range) = fake_ip_v4 {
        settings["__fake_ip_range"] = Value::String(range);
    }
    if let Some(range6) = fake_ip_v6 {
        settings["__fake_ip_range6"] = Value::String(range6);
    }
}

fn build_network_plan(
    paths: &Paths,
    settings: &Value,
    rules: &network_rules::Document,
) -> Result<Option<network::Plan>> {
    let capabilities = network::detect(paths);
    Ok(Some(network::plan_with_dns_and_rules(
        settings,
        capabilities,
        dns_listen_port(paths),
        Some(rules),
    )?))
}

pub fn preview_network_plan(paths: &Paths, rules: &network_rules::Document) -> Result<Value> {
    if !paths.settings.is_file() {
        return Ok(json!({"available": false, "reason": "settings.json is missing"}));
    }
    let store = SettingsStore::open(paths.clone())?;
    let mut settings = Value::Object(store.load()?);
    enrich_network_settings(paths, &mut settings);
    let plan = build_network_plan(paths, &settings, rules)?
        .ok_or_else(|| BoxError::new(ErrorCode::NotFound, "network settings are unavailable"))?;
    let mut chains = std::collections::BTreeMap::<String, usize>::new();
    for rule in &plan.rules {
        let key = format!("{}/{}/{}", rule.family, rule.table, rule.chain);
        *chains.entry(key).or_default() += 1;
    }
    Ok(json!({
        "available": true,
        "requested_mode": plan.requested_mode,
        "effective_mode": plan.effective_mode,
        "degraded": plan.degraded,
        "rule_count": plan.rules.len(),
        "chains": chains,
        "warnings": plan.warnings
    }))
}

pub fn apply_network_rules(
    paths: &Paths,
    previous: Option<&network_rules::Document>,
) -> Result<Value> {
    let store = SettingsStore::open(paths.clone())?;
    let mut settings = Value::Object(store.load()?);
    enrich_network_settings(paths, &mut settings);
    let rules = network_rules::load_for_plan(paths)?;
    let plan = build_network_plan(paths, &settings, &rules)?
        .ok_or_else(|| BoxError::new(ErrorCode::NotFound, "network settings are unavailable"))?;
    let mut runner = network::SystemRunner;
    cleanup_all_network(paths, &mut runner)?;
    if let Err(error) = network::apply(&plan, &mut runner) {
        let _ = network::cleanup_stale(&mut runner);
        if let Some(previous) = previous {
            if let Ok(Some(old_plan)) = build_network_plan(paths, &settings, previous) {
                let _ = network::apply(&old_plan, &mut runner);
            }
        }
        return Err(error);
    }
    apply_runtime_network_post_start(paths, &plan, &mut runner);
    Ok(json!({
        "applied": true,
        "effective_mode": plan.effective_mode,
        "rules": plan.rules.len(),
        "warnings": plan.warnings
    }))
}

fn proxy_uids(paths: &Paths, settings: &Value) -> Vec<u32> {
    if settings.get("proxy_mode").and_then(Value::as_str) == Some("core") {
        return Vec::new();
    }
    let Ok(entries) = fs::read_to_string(&paths.package_list) else {
        return Vec::new();
    };
    let Ok(packages_text) = fs::read_to_string("/data/system/packages.list") else {
        return Vec::new();
    };
    let mut packages = std::collections::BTreeMap::new();
    for line in packages_text.lines() {
        let mut fields = line.split_whitespace();
        if let (Some(package), Some(uid)) = (fields.next(), fields.next()) {
            if let Ok(uid) = uid.parse::<u32>() {
                packages.insert(package, uid);
            }
        }
    }
    let mut uids = std::collections::BTreeSet::new();
    for raw in entries.lines() {
        let entry = raw.trim();
        if entry.is_empty() || entry.starts_with('#') || entry.starts_with("mode:") {
            continue;
        }
        let (prefix, package) = match entry.split_once(':') {
            Some((prefix, package)) => (prefix.trim(), package.trim()),
            None => ("0", entry),
        };
        let prefix = prefix.parse::<u32>().unwrap_or(0);
        if let Some(uid) = packages.get(package) {
            uids.insert(uid.saturating_add(prefix.saturating_mul(100_000)));
        }
    }
    uids.into_iter().collect()
}

fn proxy_gids(paths: &Paths) -> Vec<u32> {
    fs::read_to_string(&paths.gid_list)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.parse::<u32>().ok())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn read_ap_lists(paths: &Paths) -> (Vec<String>, Vec<String>) {
    let mut allow = Vec::new();
    let mut ignore = Vec::new();
    let Ok(text) = fs::read_to_string(&paths.ap_list) else {
        return (allow, ignore);
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(action) = parts.next() else { continue };
        let Some(iface) = parts.next() else { continue };
        match action {
            "allow" => allow.push(iface.to_string()),
            "ignore" => ignore.push(iface.to_string()),
            _ => {}
        }
    }
    (allow, ignore)
}

pub fn tun_device_name(paths: &Paths) -> String {
    fs::read_to_string(&paths.config)
        .ok()
        .and_then(|text| serde_yaml::from_str::<serde_yaml::Value>(&text).ok())
        .and_then(|yaml| {
            yaml.get("tun")
                .and_then(|tun| tun.get("device"))
                .and_then(serde_yaml::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "meta".to_string())
}

fn fake_ip_ranges(paths: &Paths) -> (Option<String>, Option<String>) {
    let Ok(text) = fs::read_to_string(&paths.config) else {
        return (None, None);
    };
    let Ok(config) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return (None, None);
    };
    let Some(dns) = config.get("dns") else {
        return (None, None);
    };
    let enhanced = dns
        .get("enhanced-mode")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("fake-ip");
    if enhanced != "fake-ip" {
        return (None, None);
    }
    let v4 = dns
        .get("fake-ip-range")
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| Some("198.18.0.1/16".to_string()));
    let v6 = dns
        .get("fake-ip-range6")
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| Some("fc00::/18".to_string()));
    (v4, v6)
}

fn dns_listen_port(paths: &Paths) -> Option<u16> {
    let text = fs::read_to_string(&paths.config).ok()?;
    let config: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    if let Some(listen) = config
        .get("dns")
        .and_then(|dns| dns.get("listen"))
        .and_then(serde_yaml::Value::as_str)
    {
        if let Some(port) = listen.rsplit(':').next().and_then(|v| v.trim().parse().ok()) {
            return Some(port);
        }
    }
    Some(1053)
}

pub fn recover(paths: &Paths) -> Result<Value> {
    let Some(state) = read_service_state(paths) else {
        return Ok(json!({"phase": "idle", "recovered": false}));
    };
    let phase = state
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("idle")
        .to_string();
    if phase == "manual" || phase == "idle" || phase == "stopped" {
        return Ok(json!({"phase": phase, "recovered": false}));
    }
    let pid = read_pid(paths)?;
    let alive = pid.is_some_and(|pid| core_alive(paths, pid));
    let next = match phase.as_str() {
        "preparing" | "staged" => "idle",
        "switching" | "verifying" | "applied" => "applied",
        "rolling_back" => "manual",
        _ => "manual",
    };
    let intent = read_intent(paths);
    let explicitly_stopped = intent.as_deref() == Some("stopped");
    let next = if explicitly_stopped && next == "applied" {
        "stopped"
    } else {
        next
    };
    let error = if next == "manual" {
        Some("unfinished transaction requires manual intervention".to_string())
    } else if !alive && !explicitly_stopped {
        Some("core is not running after daemon recovery".to_string())
    } else {
        None
    };
    let state = json!({
        "txid": state.get("txid").cloned().unwrap_or(Value::Null),
        "phase": next,
        "updated_at": util::format_unix_utc(util::now_ms() / 1000),
        "last_error": error
    });
    util::ensure_dir(&paths.state_dir, true)?;
    write_json(&paths.service_state, &state)?;
    Ok(json!({
        "phase": next,
        "recovered": true,
        "core_running": alive,
        "last_error": error
    }))
}

#[derive(Debug, PartialEq, Eq)]
enum SupervisorAction {
    Idle,
    Running,
    SkipManual,
    Exhausted,
    Restart,
}

fn supervisor_action(
    intent: Option<&str>,
    running: bool,
    phase: Option<&str>,
    failures: u32,
) -> SupervisorAction {
    if intent != Some("running") {
        return SupervisorAction::Idle;
    }
    if running {
        return SupervisorAction::Running;
    }
    if phase == Some("manual") {
        return SupervisorAction::SkipManual;
    }
    if failures >= 5 {
        return SupervisorAction::Exhausted;
    }
    SupervisorAction::Restart
}

pub fn supervisor_loop(paths: Paths) {
    let mut failures = 0u32;
    loop {
        thread::sleep(Duration::from_secs(1));
        let Ok(current) = status(&paths) else {
            continue;
        };
        match supervisor_action(
            read_intent(&paths).as_deref(),
            current["running"].as_bool().unwrap_or(false),
            current["phase"].as_str(),
            failures,
        ) {
            SupervisorAction::Idle | SupervisorAction::Running => {
                failures = 0;
                continue;
            }
            SupervisorAction::SkipManual => continue,
            SupervisorAction::Exhausted => {
                let _ = write_manual_state(&paths, "supervisor retries exhausted");
                continue;
            }
            SupervisorAction::Restart => {}
        }
        let delay = backoff_seconds(failures);
        thread::sleep(Duration::from_secs(delay));
        if read_intent(&paths).as_deref() != Some("running") {
            failures = 0;
            continue;
        }
        match start_for_restart(&paths) {
            Ok(_) => {
                failures = 0;
            }
            Err(_) => {
                failures += 1;
                if failures >= 5 {
                    let _ = write_manual_state(&paths, "supervisor retries exhausted");
                }
            }
        }
    }
}

fn backoff_seconds(failures: u32) -> u64 {
    match failures {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        _ => 16,
    }
}

pub fn control(paths: &Paths) -> Value {
    let value = util::read_to_string_optional(&paths.control_state)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({"mode": "auto", "desired": "stopped"}));
    json!({
        "mode": value.get("mode").and_then(Value::as_str).unwrap_or("auto"),
        "desired": value.get("desired").and_then(Value::as_str).unwrap_or("stopped")
    })
}

pub fn write_control(paths: &Paths, mode: &str, desired: &str) -> Result<Value> {
    if !matches!(mode, "manual" | "auto") {
        return Err(BoxError::validation("control mode must be manual or auto"));
    }
    if !matches!(desired, "running" | "stopped") {
        return Err(BoxError::validation(
            "control desired must be running or stopped",
        ));
    }
    let data = json!({
        "mode": mode,
        "desired": desired,
        "updated_at": util::format_unix_utc(util::now_ms() / 1000)
    });
    util::ensure_dir(&paths.state_dir, true)?;
    util::atomic_write(
        &paths.control_state,
        serde_json::to_vec_pretty(&data)
            .map_err(|_| BoxError::internal("cannot serialize control state"))?
            .as_slice(),
        0o600,
    )?;
    Ok(data)
}

fn set_intent(paths: &Paths, intent: &str) -> Result<()> {
    util::ensure_dir(&paths.state_dir, true)?;
    util::atomic_write(&paths.service_intent, intent.as_bytes(), 0o600)
}

fn read_intent(paths: &Paths) -> Option<String> {
    util::read_to_string_optional(&paths.service_intent)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
}

fn write_manual_state(paths: &Paths, message: &str) -> Result<()> {
    let state = json!({
        "txid": Value::Null,
        "phase": "manual",
        "updated_at": util::format_unix_utc(util::now_ms() / 1000),
        "last_error": message
    });
    write_json(&paths.service_state, &state)
}

fn selected_mode(paths: &Paths) -> Option<String> {
    let store = SettingsStore::open(paths.clone()).ok()?;
    let settings = store.load().ok()?;
    settings
        .get("network_mode")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn yaml_mapping(value: &serde_yaml::Value) -> Option<&serde_yaml::Mapping> {
    value.as_mapping()
}

fn yaml_key(key: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(key.to_string())
}

fn yaml_insert_missing(
    mapping: &mut serde_yaml::Mapping,
    key: &str,
    value: serde_yaml::Value,
) -> bool {
    let key = yaml_key(key);
    if mapping.contains_key(&key) {
        false
    } else {
        mapping.insert(key, value);
        true
    }
}

fn proxy_lists_configured(paths: &Paths) -> bool {
    let has_pkg_or_gid = [&paths.package_list, &paths.gid_list].into_iter().any(|path| {
        fs::read_to_string(path)
            .map(|text| {
                text.lines().any(|line| {
                    let line = line.trim();
                    !line.is_empty() && !line.starts_with('#') && !line.starts_with("mode:")
                })
            })
            .unwrap_or(false)
    });
    if has_pkg_or_gid {
        return true;
    }
    let (_, ignore_aps) = read_ap_lists(paths);
    !ignore_aps.is_empty()
}

fn proxy_uid_values(paths: &Paths, settings: &Value) -> Vec<u64> {
    let mut values = proxy_uids(paths, settings)
        .into_iter()
        .map(u64::from)
        .collect::<Vec<_>>();
    values.extend(proxy_gids(paths).into_iter().map(u64::from));
    values.sort_unstable();
    values.dedup();
    values
}

fn prepare_tun_block(
    mapping: &mut serde_yaml::Mapping,
    paths: &Paths,
    settings: &Value,
    mode: &str,
) -> bool {
    if !matches!(mode, "tun" | "mixed") {
        return false;
    }
    let tun_key = yaml_key("tun");
    let existed = mapping.contains_key(&tun_key);
    let mut tun = mapping
        .get(&tun_key)
        .and_then(yaml_mapping)
        .cloned()
        .unwrap_or_default();
    let cnip = settings
        .get("bypass_cn_ip")
        .and_then(Value::as_str)
        .map(|value| value == "true")
        .unwrap_or(false);
    let proxy_mode = settings
        .get("proxy_mode")
        .and_then(Value::as_str)
        .unwrap_or("core");
    let changed_before = tun.clone();

    tun.insert(yaml_key("enable"), serde_yaml::Value::Bool(true));
    yaml_insert_missing(&mut tun, "mtu", serde_yaml::Value::Number(9000u64.into()));
    yaml_insert_missing(
        &mut tun,
        "device",
        serde_yaml::Value::String("meta".to_string()),
    );
    yaml_insert_missing(
        &mut tun,
        "dns-hijack",
        serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("any:53".to_string()),
            serde_yaml::Value::String("tcp://any:53".to_string()),
        ]),
    );
    yaml_insert_missing(
        &mut tun,
        "include-android-user",
        serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::Number(0u64.into()),
            serde_yaml::Value::Number(10u64.into()),
        ]),
    );
    if cnip {
        // box.iptables owns policy routing when CNIP bypass is active.
        tun.insert(
            yaml_key("stack"),
            serde_yaml::Value::String("gvisor".to_string()),
        );
        for key in [
            "auto-route",
            "strict-route",
            "auto-redirect",
            "auto-detect-interface",
        ] {
            tun.insert(yaml_key(key), serde_yaml::Value::Bool(false));
        }
    } else {
        yaml_insert_missing(
            &mut tun,
            "stack",
            serde_yaml::Value::String("mixed".to_string()),
        );
        tun.insert(yaml_key("auto-route"), serde_yaml::Value::Bool(true));
        tun.insert(
            yaml_key("auto-detect-interface"),
            serde_yaml::Value::Bool(true),
        );
        yaml_insert_missing(&mut tun, "strict-route", serde_yaml::Value::Bool(false));
        yaml_insert_missing(&mut tun, "auto-redirect", serde_yaml::Value::Bool(false));
    }

    if proxy_mode == "core" {
        tun.insert(
            yaml_key("include-uid"),
            serde_yaml::Value::Sequence(Vec::new()),
        );
        tun.insert(
            yaml_key("exclude-uid"),
            serde_yaml::Value::Sequence(Vec::new()),
        );
    } else {
        let uids = proxy_uid_values(paths, settings)
            .into_iter()
            .map(|uid| serde_yaml::Value::Number(uid.into()))
            .collect::<Vec<_>>();
        if proxy_mode == "whitelist" {
            tun.insert(yaml_key("include-uid"), serde_yaml::Value::Sequence(uids));
            tun.insert(
                yaml_key("exclude-uid"),
                serde_yaml::Value::Sequence(Vec::new()),
            );
        } else {
            tun.insert(yaml_key("exclude-uid"), serde_yaml::Value::Sequence(uids));
            tun.insert(
                yaml_key("include-uid"),
                serde_yaml::Value::Sequence(Vec::new()),
            );
        }
    }

    let changed = !existed || changed_before != tun;
    if changed {
        mapping.insert(tun_key, serde_yaml::Value::Mapping(tun));
    }
    changed
}

fn prepare_dns_block(
    mapping: &mut serde_yaml::Mapping,
    paths: &Paths,
    settings: &Value,
    mode: &str,
) -> bool {
    let dns_key = yaml_key("dns");
    if !mapping.contains_key(&dns_key) {
        return false;
    }
    let mut dns = mapping
        .get(&dns_key)
        .and_then(yaml_mapping)
        .cloned()
        .unwrap_or_default();
    let before = dns.clone();
    yaml_insert_missing(
        &mut dns,
        "enhanced-mode",
        serde_yaml::Value::String("fake-ip".to_string()),
    );
    let proxy_mode = settings
        .get("proxy_mode")
        .and_then(Value::as_str)
        .unwrap_or("core");
    let enhanced = dns
        .get(yaml_key("enhanced-mode"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("fake-ip");
    let should_switch = mode != "tun"
        && proxy_mode != "core"
        && proxy_lists_configured(paths)
        && enhanced == "fake-ip";
    let mut changed = before != dns;
    if should_switch {
        dns.insert(
            yaml_key("enhanced-mode"),
            serde_yaml::Value::String("redir-host".to_string()),
        );
        changed = true;
    }
    if changed {
        mapping.insert(dns_key, serde_yaml::Value::Mapping(dns));
    }
    if should_switch {
        // Mihomo's `sniffer:` is a top-level section, NOT nested under `dns:`.
        let sniffer_key = yaml_key("sniffer");
        let mut sniffer = mapping
            .get(&sniffer_key)
            .and_then(yaml_mapping)
            .cloned()
            .unwrap_or_default();
        if sniffer
            .get(yaml_key("enable"))
            .and_then(serde_yaml::Value::as_bool)
            != Some(true)
        {
            sniffer.insert(yaml_key("enable"), serde_yaml::Value::Bool(true));
            mapping.insert(sniffer_key, serde_yaml::Value::Mapping(sniffer));
            changed = true;
        }
    }
    changed
}

fn auto_modify_config_enabled(paths: &Paths) -> bool {
    SettingsStore::open(paths.clone())
        .ok()
        .and_then(|store| store.load().ok())
        .and_then(|settings| settings.get("auto_modify_config").cloned())
        .and_then(|value| value.as_str().map(|value| value == "true"))
        .unwrap_or(true)
}

pub fn find_process_mode(paths: &Paths) -> String {
    SettingsStore::open(paths.clone())
        .ok()
        .and_then(|store| store.load().ok())
        .and_then(|settings| settings.get("find_process_mode").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| matches!(value.as_str(), "always" | "strict" | "off"))
        .unwrap_or_else(|| "off".to_string())
}

fn prepare_candidate(paths: &Paths, candidate: &Path, mode: Option<&str>) -> Result<()> {
    // Tests build a deliberately minimal HOME without settings.json. Real
    // installations always have it, and only they need the legacy config
    // preparation (controller/UI/ports/TUN).
    if !paths.settings.is_file() {
        return Ok(());
    }
    let store = SettingsStore::open(paths.clone())?;
    let settings = store.load()?;
    if settings
        .get("auto_modify_config")
        .and_then(Value::as_str)
        .is_some_and(|value| value != "true")
    {
        return Ok(());
    }
    let text = fs::read_to_string(candidate)?;
    let mut value: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        BoxError::validation("core config is not valid YAML").with_detail(error.to_string())
    })?;
    let Some(mapping) = value.as_mapping_mut() else {
        return Err(BoxError::validation("core config must be a mapping"));
    };
    let mut changed = false;
    let ensure = |mapping: &mut serde_yaml::Mapping, key: &str, value: serde_yaml::Value| {
        let key = serde_yaml::Value::String(key.to_string());
        if !mapping.contains_key(&key) {
            mapping.insert(key, value);
            true
        } else {
            false
        }
    };
    changed |= ensure(
        mapping,
        "external-controller",
        serde_yaml::Value::String("0.0.0.0:9090".to_string()),
    );
    changed |= ensure(
        mapping,
        "external-ui",
        serde_yaml::Value::String("./dashboard".to_string()),
    );
    let process_mode = find_process_mode(paths);
    let process_key = serde_yaml::Value::String("find-process-mode".to_string());
    if mapping
        .get(&process_key)
        .and_then(serde_yaml::Value::as_str)
        != Some(process_mode.as_str())
    {
        mapping.insert(process_key, serde_yaml::Value::String(process_mode.clone()));
        changed = true;
    }
    for (key, setting, fallback) in [
        ("tproxy-port", "tproxy_port", 9898u64),
        ("redir-port", "redir_port", 9797u64),
    ] {
        let port = settings
            .get(setting)
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(fallback);
        let key = serde_yaml::Value::String(key.to_string());
        let wanted = serde_yaml::Value::Number(port.into());
        if mapping.get(&key) != Some(&wanted) {
            mapping.insert(key, wanted);
            changed = true;
        }
    }
    if let Some(mode) = mode {
        let settings_value = Value::Object(settings.clone());
        changed |= prepare_tun_block(mapping, paths, &settings_value, mode);
        changed |= prepare_dns_block(mapping, paths, &settings_value, mode);
    }
    if changed {
        let bytes = serde_yaml::to_string(&value).map_err(|error| {
            BoxError::internal("cannot serialize core config").with_detail(error.to_string())
        })?;
        util::atomic_write(candidate, bytes.as_bytes(), 0o600)?;
    }
    Ok(())
}

fn enable_tun_candidate(paths: &Paths, candidate: &Path) -> Result<()> {
    if !auto_modify_config_enabled(paths) {
        return Ok(());
    }
    let text = fs::read_to_string(candidate)?;
    let mut value: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        BoxError::validation("core config is not valid YAML").with_detail(error.to_string())
    })?;
    let Some(mapping) = value.as_mapping_mut() else {
        return Err(BoxError::validation("core config must be a mapping"));
    };
    let tun_key = serde_yaml::Value::String("tun".to_string());
    let mut tun = mapping
        .get(&tun_key)
        .and_then(serde_yaml::Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    tun.insert(
        serde_yaml::Value::String("enable".to_string()),
        serde_yaml::Value::Bool(true),
    );
    mapping.insert(tun_key, serde_yaml::Value::Mapping(tun));
    mapping.insert(
        serde_yaml::Value::String("find-process-mode".to_string()),
        serde_yaml::Value::String(find_process_mode(paths)),
    );
    let bytes = serde_yaml::to_string(&value).map_err(|error| {
        BoxError::internal("cannot serialize core config").with_detail(error.to_string())
    })?;
    util::atomic_write(candidate, bytes.as_bytes(), 0o600)
}

fn disable_tun_candidate(paths: &Paths, candidate: &Path) -> Result<()> {
    let auto_modify = SettingsStore::open(paths.clone())
        .ok()
        .and_then(|store| store.load().ok())
        .and_then(|settings| settings.get("auto_modify_config").cloned())
        .and_then(|value| value.as_str().map(|value| value == "true"))
        .unwrap_or(true);
    if !auto_modify {
        return Ok(());
    }
    let text = fs::read_to_string(candidate)
        .map_err(|error| BoxError::from(error).with_detail("cannot read candidate config"))?;
    let mut value: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        BoxError::validation("candidate config is not valid YAML").with_detail(error.to_string())
    })?;
    let Some(mapping) = value.as_mapping_mut() else {
        return Ok(());
    };
    let key = serde_yaml::Value::String("tun".to_string());
    let mut changed = false;
    if let Some(tun) = mapping
        .get_mut(&key)
        .and_then(serde_yaml::Value::as_mapping_mut)
    {
        tun.insert(
            serde_yaml::Value::String("enable".to_string()),
            serde_yaml::Value::Bool(false),
        );
        changed = true;
    }
    let process_mode = find_process_mode(paths);
    let process_key = serde_yaml::Value::String("find-process-mode".to_string());
    if mapping.contains_key(&process_key) {
        mapping.insert(process_key, serde_yaml::Value::String(process_mode));
        changed = true;
    }
    if changed {
        let bytes = serde_yaml::to_string(&value).map_err(|error| {
            BoxError::internal("cannot serialize prepared candidate").with_detail(error.to_string())
        })?;
        util::atomic_write(candidate, bytes.as_bytes(), 0o600)?;
    }
    Ok(())
}

fn begin_tx(paths: &Paths, action: &str) -> Result<Tx> {
    util::ensure_dir(&paths.state_dir, true)?;
    util::ensure_dir(&paths.tx_dir, true)?;
    let txid = format!("{}", util::now_ms());
    let txid = format!("{txid}-{}", std::process::id());
    let dir = paths.tx_dir.join(&txid);
    util::ensure_dir(&dir, true)?;
    let candidate = if action == "start" {
        let candidate_dir = paths.candidates_dir.join(&txid);
        util::ensure_dir(&candidate_dir, true)?;
        let candidate = candidate_dir.join("config.yaml");
        fs::copy(&paths.config, &candidate)
            .map_err(|error| BoxError::from(error).with_detail("cannot stage candidate config"))?;
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o600))
            .map_err(|error| BoxError::from(error).with_detail("cannot chmod candidate config"))?;
        candidate
    } else {
        paths.config.clone()
    };
    let config_hash = fs::read(&candidate)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .unwrap_or_default();
    let manifest = json!({
        "txid": txid,
        "action": action,
        "created_at": util::format_unix_utc(util::now_ms() / 1000),
        "config_sha256": config_hash,
        "candidate": candidate.display().to_string()
    });
    write_json(&dir.join("manifest.json"), &manifest)?;
    Ok(Tx {
        txid,
        dir,
        candidate,
    })
}

fn set_phase(paths: &Paths, tx: &Tx, phase: &str, error: Option<&str>) -> Result<()> {
    let state = json!({
        "txid": tx.txid,
        "phase": phase,
        "updated_at": util::format_unix_utc(util::now_ms() / 1000),
        "last_error": error
    });
    write_json(&tx.dir.join("state.json"), &state)?;
    write_json(&paths.service_state, &state)?;
    Ok(())
}

fn fail_manual(paths: &Paths, tx: &Tx, message: &str) -> Result<()> {
    set_phase(paths, tx, "manual", Some(message))
}

fn write_last_good(paths: &Paths, tx: &Tx) -> Result<()> {
    let data = json!({
        "txid": tx.txid,
        "updated_at": util::format_unix_utc(util::now_ms() / 1000)
    });
    write_json(&paths.last_good, &data)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        BoxError::internal("cannot serialize service state").with_detail(error.to_string())
    })?;
    util::atomic_write(path, &bytes, 0o600)
}

fn read_service_state(paths: &Paths) -> Option<Value> {
    let text = util::read_to_string_optional(&paths.service_state)
        .ok()
        .flatten()?;
    serde_json::from_str(&text).ok()
}

fn check_candidate(paths: &Paths, config: &Path) -> bool {
    let output = Command::new(&paths.core_bin)
        .arg("-t")
        .arg("-d")
        .arg(paths.home.join("mihomo"))
        .arg("-f")
        .arg(config)
        .output();
    match output {
        Ok(output) => {
            let _ = util::atomic_write(&paths.core_log, &output.stdout, 0o600);
            output.status.success()
        }
        Err(_) => false,
    }
}

fn spawn_core(paths: &Paths, config: &Path) -> Result<std::process::Child> {
    let log = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&paths.core_log)
        .map_err(|error| BoxError::from(error).with_detail("cannot open core log"))?;
    let error_log = log
        .try_clone()
        .map_err(|error| BoxError::from(error).with_detail("cannot clone core log"))?;
    let cpu_list = taskset_cpus(paths);
    let mut command = Command::new(&paths.core_bin);
    command
        .current_dir(paths.home.join("mihomo"))
        .arg("-d")
        .arg(paths.home.join("mihomo"))
        .arg("-f")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log));
    unsafe {
        command.pre_exec(move || {
            // Do not leak daemon control sockets or event fds into the Go core.
            for fd in 3..256 {
                libc::close(fd);
            }
            // Raise RLIMIT_NOFILE toward 1_000_000 (legacy ulimit -SHn 1000000); degrade cleanly if denied.
            for target in [CORE_NOFILE_LIMIT, 262_144, 65_536] {
                let lim = libc::rlimit {
                    rlim_cur: target as libc::rlim_t,
                    rlim_max: target as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &lim) == 0 {
                    break;
                }
            }
            // Run mihomo under gid=3005 (Android net_admin) when running as root so
            // iptables `--uid-owner 0 --gid-owner 3005` bypasses only the core itself.
            if libc::geteuid() == 0 {
                let _ = libc::setgid(NET_ADMIN_GID as libc::gid_t);
            }
            // Apply CPU affinity if taskset_cpu=true and allow_cpu parsed successfully.
            if let Some(ref cpus) = cpu_list {
                let mut set: libc::cpu_set_t = std::mem::zeroed();
                libc::CPU_ZERO(&mut set);
                for &cpu in cpus {
                    libc::CPU_SET(cpu, &mut set);
                }
                let _ = libc::sched_setaffinity(
                    0,
                    std::mem::size_of::<libc::cpu_set_t>(),
                    &set,
                );
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|error| BoxError::from(error).with_detail("cannot spawn core"))
}

fn taskset_cpus(paths: &Paths) -> Option<Vec<usize>> {
    let store = SettingsStore::open(paths.clone()).ok()?;
    let settings = store.load().ok()?;
    if settings.get("taskset_cpu").and_then(Value::as_str) != Some("true") {
        return None;
    }
    let spec = settings
        .get("allow_cpu")
        .and_then(Value::as_str)
        .unwrap_or_default();
    parse_cpu_list(spec)
}

pub fn parse_cpu_list(spec: &str) -> Option<Vec<usize>> {
    let mut cpus = std::collections::BTreeSet::new();
    for item in spec.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if let Some((start, end)) = item.split_once('-') {
            let start = start.trim().parse::<usize>().ok()?;
            let end = end.trim().parse::<usize>().ok()?;
            if start > end || end >= 1024 {
                return None;
            }
            for cpu in start..=end {
                cpus.insert(cpu);
            }
        } else {
            let cpu = item.parse::<usize>().ok()?;
            if cpu >= 1024 {
                return None;
            }
            cpus.insert(cpu);
        }
    }
    if cpus.is_empty() {
        None
    } else {
        Some(cpus.into_iter().collect())
    }
}

pub fn parse_memcg_limit(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (digits, mult) = match raw.as_bytes().last().copied() {
        Some(b'k' | b'K') => (&raw[..raw.len() - 1], 1024u64),
        Some(b'm' | b'M') => (&raw[..raw.len() - 1], 1024u64 * 1024),
        Some(b'g' | b'G') => (&raw[..raw.len() - 1], 1024u64 * 1024 * 1024),
        _ => (raw, 1u64),
    };
    digits.trim().parse::<u64>().ok()?.checked_mul(mult)
}

fn find_cgroup_mount(controller: &str) -> Option<(std::path::PathBuf, bool)> {
    let mounts = fs::read_to_string("/proc/mounts").ok()?;
    // Prefer cgroup v1 dedicated mount matching controller name.
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let _dev = fields.next()?;
        let mount_point = fields.next()?;
        let fstype = fields.next()?;
        let options = fields.next()?;
        if fstype == "cgroup"
            && options
                .split(',')
                .any(|opt| opt == controller || (controller == "memory" && opt == "memcg"))
        {
            return Some((std::path::PathBuf::from(mount_point), false));
        }
    }
    // Fallback: cgroup2 unified mount if controller is listed in cgroup.controllers.
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let _dev = fields.next()?;
        let mount_point = fields.next()?;
        let fstype = fields.next()?;
        if fstype == "cgroup2" {
            let path = std::path::PathBuf::from(mount_point);
            let controllers = fs::read_to_string(path.join("cgroup.controllers")).unwrap_or_default();
            let v2_name = if controller == "blkio" { "io" } else { controller };
            if controllers.split_whitespace().any(|c| c == v2_name) {
                return Some((path, true));
            }
        }
    }
    None
}

fn apply_cgroups_for_pid(paths: &Paths, pid: u32) -> Vec<String> {
    let mut warnings = Vec::new();
    let Ok(store) = SettingsStore::open(paths.clone()) else {
        return warnings;
    };
    let Ok(settings) = store.load() else {
        return warnings;
    };
    if settings.get("cgroup_memcg").and_then(Value::as_str) == Some("true") {
        let raw_limit = settings
            .get("memcg_limit")
            .and_then(Value::as_str)
            .unwrap_or("100M");
        if let Err(msg) = apply_memcg(pid, raw_limit) {
            warnings.push(format!("cgroup_memcg degraded: {msg}"));
        }
    }
    if settings.get("cgroup_blkio").and_then(Value::as_str) == Some("true") {
        let raw_weight = settings
            .get("weight")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("900");
        if let Err(msg) = apply_blkio(pid, raw_weight) {
            warnings.push(format!("cgroup_blkio degraded: {msg}"));
        }
    }
    warnings
}

fn apply_memcg(pid: u32, raw_limit: &str) -> std::result::Result<(), String> {
    let limit = parse_memcg_limit(raw_limit)
        .ok_or_else(|| format!("invalid memcg_limit: {raw_limit}"))?;
    let (mount, is_v2) = find_cgroup_mount("memory")
        .ok_or_else(|| "memory cgroup mount not found".to_string())?;
    let mut target = mount.join("box");
    if !target.is_dir() && fs::create_dir_all(&target).is_err() {
        target = mount.join("mihomo");
        fs::create_dir_all(&target)
            .map_err(|e| format!("cannot create memory cgroup dir {}: {e}", target.display()))?;
    }
    let limit_file = if is_v2 {
        target.join("memory.max")
    } else {
        target.join("memory.limit_in_bytes")
    };
    fs::write(&limit_file, format!("{limit}\n"))
        .map_err(|e| format!("cannot write {}: {e}", limit_file.display()))?;
    fs::write(target.join("cgroup.procs"), format!("{pid}\n"))
        .map_err(|e| format!("cannot assign pid {pid} to {}: {e}", target.display()))?;
    Ok(())
}

fn apply_blkio(pid: u32, raw_weight: &str) -> std::result::Result<(), String> {
    let weight: u32 = raw_weight
        .trim()
        .parse()
        .map_err(|_| format!("invalid blkio weight: {raw_weight}"))?;
    let (mount, is_v2) = find_cgroup_mount("blkio")
        .ok_or_else(|| "blkio/io cgroup mount not found".to_string())?;
    let mut target = mount.join("box");
    let mut created_own = true;
    if !target.is_dir() && fs::create_dir_all(&target).is_err() {
        created_own = false;
        if mount.join("foreground").is_dir() {
            target = mount.join("foreground");
        } else if mount.join("top-app").is_dir() {
            target = mount.join("top-app");
        } else {
            return Err(format!("cannot create blkio cgroup dir {}", target.display()));
        }
    }
    if created_own {
        let weight_file = if is_v2 {
            target.join("io.weight")
        } else {
            target.join("blkio.weight")
        };
        let content = if is_v2 {
            format!("default {weight}\n")
        } else {
            format!("{weight}\n")
        };
        let _ = fs::write(&weight_file, content);
    }
    fs::write(target.join("cgroup.procs"), format!("{pid}\n"))
        .map_err(|e| format!("cannot assign pid {pid} to {}: {e}", target.display()))?;
    Ok(())
}

fn cleanup_old_candidates(paths: &Paths, keep: usize) {
    let Ok(entries) = fs::read_dir(&paths.candidates_dir) else {
        return;
    };
    let mut dirs = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let mtime = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (mtime, entry.path())
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

fn kill_orphan_cores(paths: &Paths, exclude_pid: Option<u32>) {
    let Ok(entries) = fs::read_dir("/proc") else {
        return;
    };
    let my_pid = std::process::id();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == my_pid || exclude_pid == Some(pid) {
            continue;
        }
        if core_alive(paths, pid) {
            terminate(pid, libc::SIGTERM);
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(500) && core_alive(paths, pid) {
                thread::sleep(Duration::from_millis(50));
            }
            if core_alive(paths, pid) {
                terminate(pid, libc::SIGKILL);
            }
        }
    }
}

fn wait_tun_device(device: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    let sys_path = Path::new("/sys/class/net").join(device);
    while started.elapsed() < timeout {
        if sys_path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    sys_path.exists()
}

fn load_cnip_ipsets_if_needed(paths: &Paths) -> Vec<String> {
    let mut warnings = Vec::new();
    let Ok(store) = SettingsStore::open(paths.clone()) else {
        return warnings;
    };
    let Ok(settings) = store.load() else {
        return warnings;
    };
    if settings.get("bypass_cn_ip").and_then(Value::as_str) != Some("true") {
        return warnings;
    }
    let caps = network::detect(paths);
    if !caps.ipset {
        return warnings;
    }
    let ipv6 = settings.get("ipv6").and_then(Value::as_str) == Some("true");
    let v4 = settings.get("bypass_cn_ip_v4").and_then(Value::as_str) == Some("true");
    let v6 = ipv6 && settings.get("bypass_cn_ip_v6").and_then(Value::as_str) == Some("true");
    if v4 && paths.cnip_v4_file.is_file() {
        if let Err(err) = crate::task::reload_cnip_set(&paths.cnip_v4_file, "cnip", "inet") {
            warnings.push(format!("cnip v4 ipset load failed: {err}"));
        }
    }
    if v6 && paths.cnip_v6_file.is_file() {
        if let Err(err) = crate::task::reload_cnip_set(&paths.cnip_v6_file, "cnip6", "inet6") {
            warnings.push(format!("cnip v6 ipset load failed: {err}"));
        }
    }
    warnings
}

fn apply_runtime_network_post_start(
    paths: &Paths,
    plan: &network::Plan,
    runner: &mut dyn network::Runner,
) {
    let Ok(store) = SettingsStore::open(paths.clone()) else {
        return;
    };
    let Ok(settings) = store.load() else {
        return;
    };
    let ipv6 = settings.get("ipv6").and_then(Value::as_str) == Some("true");
    if !ipv6 {
        network::disable_ipv6_policy(runner);
    } else {
        network::restore_ipv6_policy(runner);
    }
    if matches!(plan.effective_mode.as_str(), "tun" | "mixed") {
        let tun_dev = tun_device_name(paths);
        network::apply_tun_sysctl(&tun_dev, runner);
    }
    let _ = network::refresh_local_ip_chains(paths, runner);
}

fn write_pid(paths: &Paths, pid: u32) -> Result<()> {
    util::atomic_write(&paths.box_pid, format!("{pid}\n").as_bytes(), 0o600)
}

fn read_pid(paths: &Paths) -> Result<Option<u32>> {
    Ok(util::read_to_string_optional(&paths.box_pid)?
        .and_then(|value| value.trim().parse::<u32>().ok()))
}

fn core_alive(paths: &Paths, pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let result = unsafe {
        // SAFETY: signal 0 only probes whether the PID exists; the PID comes
        // from a root-owned pid file and no memory is dereferenced.
        libc::kill(pid as libc::pid_t, 0)
    };
    if result != 0 {
        return false;
    }
    let expected = fs::canonicalize(&paths.core_bin).unwrap_or_else(|_| paths.core_bin.clone());
    let exe_matches = fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|exe| fs::canonicalize(&exe).unwrap_or(exe) == expected)
        .unwrap_or(false);
    let argv_matches = fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .is_some_and(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter(|value| !value.is_empty())
                .any(|value| {
                    let arg =
                        std::path::PathBuf::from(std::ffi::OsString::from_vec(value.to_vec()));
                    fs::canonicalize(&arg).unwrap_or(arg) == expected
                })
        });
    if !exe_matches && !argv_matches {
        return false;
    }
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    let Some(close) = stat.rfind(')') else {
        return true;
    };
    !stat[close + 1..].trim_start().starts_with('Z')
}

fn reap_child(pid: u32) {
    // SIGTERM/SIGKILL can make the child waitable a few scheduler ticks later.
    for _ in 0..50 {
        let result = unsafe {
            // SAFETY: waitpid only inspects the managed PID and writes no memory.
            libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG)
        };
        if result == pid as libc::pid_t || result == -1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn terminate(pid: u32, signal: libc::c_int) {
    unsafe {
        // SAFETY: kill delivers a signal to a PID read from the managed pid
        // file; no memory is dereferenced.
        libc::kill(pid as libc::pid_t, signal);
    }
}

#[cfg(test)]
mod tests {
    static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::{
        backoff_seconds, health_check, recover, reload, reload_url, start_for_restart,
        start_with_window, status, stop,
    };
    use crate::network::Runner;
    use crate::paths::Paths;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn reload_url_preserves_scheme() {
        assert_eq!(
            reload_url("127.0.0.1:9090").expect("http"),
            "http://127.0.0.1:9090/configs?force=true"
        );
        assert_eq!(
            reload_url("https://127.0.0.1:9090").expect("https"),
            "https://127.0.0.1:9090/configs?force=true"
        );
        assert!(reload_url("https://").is_err());
    }

    #[test]
    fn recover_keeps_stopped_state_without_manual_error() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::create_dir_all(&paths.state_dir).expect("state");
        fs::write(
            &paths.service_state,
            r#"{"phase":"stopped","txid":"t1","last_error":null}"#,
        )
        .expect("service state");
        fs::write(&paths.service_intent, "stopped").expect("intent");
        let recovered = recover(&paths).expect("recover");
        assert_eq!(recovered["phase"], "stopped");
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.service_state).expect("state"))
                .expect("json");
        assert_eq!(state["phase"], "stopped");
        assert!(state["last_error"].is_null());
        let _ = fs::remove_dir_all(root);
    }

    struct NoopRunner;

    impl Runner for NoopRunner {
        fn run(&mut self, _command: &str, _args: &[String]) -> crate::error::Result<()> {
            Ok(())
        }
    }

    struct FailApplyRunner {
        failed: bool,
    }

    impl Runner for FailApplyRunner {
        fn run(&mut self, _command: &str, args: &[String]) -> crate::error::Result<()> {
            if !self.failed && args.iter().any(|arg| arg == "-A") {
                self.failed = true;
                return Err(crate::error::BoxError::new(
                    crate::error::ErrorCode::Command,
                    "injected apply failure",
                ));
            }
            Ok(())
        }
    }

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-service-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::create_dir_all(root.join("mihomo")).expect("mihomo");
        fs::create_dir_all(root.join("run")).expect("run");
        fs::write(root.join("mihomo/config.yaml"), "mode: rule\\n").expect("config");
        let script = r#"#!/bin/sh
if [ "$1" = "-t" ]; then exit 0; fi
trap 'exit 0' TERM INT
while :; do sleep 0.1; done
"#;
        fs::write(root.join("bin/mihomo"), script).expect("bin");
        fs::set_permissions(root.join("bin/mihomo"), fs::Permissions::from_mode(0o755))
            .expect("chmod");
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
        root
    }

    #[test]
    fn candidate_preparation_adds_controller_ports_and_tun() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(root.join("settings.json"), r#"{"network_mode":"mixed"}"#).expect("settings");
        let candidate = root.join("candidate.yaml");
        fs::write(&candidate, "mode: rule\n").expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("mixed")).expect("prepare");
        let text = fs::read_to_string(&candidate).expect("read");
        assert!(text.contains("external-controller"), "{text}");
        assert!(text.contains("tproxy-port"), "{text}");
        assert!(text.contains("redir-port"), "{text}");
        assert!(text.contains("tun:"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mixed_candidate_keeps_tun_and_uses_default_off_process_mode() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let candidate = root.join("candidate.yaml");
        fs::write(
            &candidate,
            "tun:\n  enable: false\nfind-process-mode: always\n",
        )
        .expect("candidate");
        super::enable_tun_candidate(&paths, &candidate).expect("enable mixed tun");
        let prepared = fs::read_to_string(&candidate).expect("read");
        assert!(prepared.contains("enable: true"), "{prepared}");
        assert!(prepared.contains("find-process-mode: off"), "{prepared}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn candidate_disables_tun_and_process_scanner_for_non_tun_modes() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let candidate = root.join("candidate.yaml");
        fs::write(
            &candidate,
            "tun:\n  enable: true\nfind-process-mode: always\n",
        )
        .expect("candidate");
        super::disable_tun_candidate(&paths, &candidate).expect("prepare candidate");
        let prepared = fs::read_to_string(&candidate).expect("read candidate");
        assert!(prepared.contains("enable: false"), "{prepared}");
        assert!(prepared.contains("find-process-mode: off"), "{prepared}");
        assert!(
            !prepared.contains("find-process-mode: always"),
            "{prepared}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tun_block_is_completed_with_legacy_fields() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(root.join("settings.json"), r#"{"network_mode":"tun"}"#).expect("settings");
        let candidate = root.join("candidate.yaml");
        fs::write(&candidate, "mode: rule\n").expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("tun")).expect("prepare");
        let text = fs::read_to_string(&candidate).expect("read");
        for expected in [
            "mtu: 9000",
            "device: meta",
            "stack: mixed",
            "dns-hijack:",
            "auto-route: true",
            "strict-route: false",
            "auto-detect-interface: true",
            "include-uid: []",
            "exclude-uid: []",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn existing_tun_block_preserves_custom_stack_and_mtu() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(root.join("settings.json"), r#"{"network_mode":"tun"}"#).expect("settings");
        let candidate = root.join("candidate.yaml");
        fs::write(
            &candidate,
            "tun:\n  enable: false\n  stack: system\n  mtu: 1500\n  auto-route: false\n",
        )
        .expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("tun")).expect("prepare");
        let text = fs::read_to_string(&candidate).expect("read");
        assert!(text.contains("enable: true"), "{text}");
        assert!(text.contains("stack: system"), "{text}");
        assert!(text.contains("mtu: 1500"), "{text}");
        assert!(text.contains("auto-route: true"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_core_dns_switches_to_redir_host_with_top_level_sniffer() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(
            root.join("settings.json"),
            r#"{"network_mode":"tproxy","proxy_mode":"blacklist"}"#,
        )
        .expect("settings");
        fs::write(root.join("package.list.cfg"), "com.example.app\n").expect("packages");
        let candidate = root.join("candidate.yaml");
        fs::write(&candidate, "dns:\n  enhanced-mode: fake-ip\n").expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("tproxy")).expect("prepare");
        let text = fs::read_to_string(&candidate).expect("read");
        let yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("yaml");
        assert_eq!(yaml["dns"]["enhanced-mode"].as_str(), Some("redir-host"));
        // sniffer MUST be top-level, NOT nested under dns.
        assert!(yaml["dns"].get("sniffer").is_none(), "{text}");
        assert_eq!(yaml["sniffer"]["enable"].as_bool(), Some(true), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_modify_config_false_leaves_candidate_untouched() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(
            root.join("settings.json"),
            r#"{"auto_modify_config":"false","network_mode":"mixed","tproxy_port":"9999"}"#,
        )
        .expect("settings");
        let candidate = root.join("candidate.yaml");
        let original = "mode: rule\ntun:\n  enable: false\n";
        fs::write(&candidate, original).expect("candidate");
        super::enable_tun_candidate(&paths, &candidate).expect("enable tun");
        super::prepare_candidate(&paths, &candidate, Some("mixed")).expect("prepare");
        assert_eq!(fs::read_to_string(&candidate).expect("read"), original);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn allow_only_ap_list_does_not_trigger_redir_host() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(
            root.join("settings.json"),
            r#"{"network_mode":"tproxy","proxy_mode":"blacklist"}"#,
        )
        .expect("settings");
        fs::write(root.join("ap.list.cfg"), "allow wlan+\nallow ap+\n").expect("ap list");
        let candidate = root.join("candidate.yaml");
        fs::write(&candidate, "dns:\n  enhanced-mode: fake-ip\n").expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("tproxy")).expect("prepare");
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&fs::read_to_string(&candidate).expect("read")).expect("yaml");
        assert_eq!(yaml["dns"]["enhanced-mode"].as_str(), Some("fake-ip"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stop_aborts_and_keeps_core_when_cleanup_fails() {
        struct FailCleanupRunner;
        impl Runner for FailCleanupRunner {
            fn run(&mut self, _command: &str, _args: &[String]) -> crate::error::Result<()> {
                Err(crate::error::BoxError::new(
                    crate::error::ErrorCode::Command,
                    "Operation not permitted",
                ))
            }
        }
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(
            root.join("settings.json"),
            r#"{"network_mode":"tproxy","auto_modify_config":"false"}"#,
        )
        .expect("settings");
        let _guard = PATH_LOCK.lock().expect("path lock");
        let fake_bin = root.join("fake-bin");
        fs::create_dir_all(&fake_bin).expect("fake bin");
        for name in ["iptables", "ip6tables"] {
            let path = fake_bin.join(name);
            fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fake command");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fake_bin.display(), old_path));
        start_with_window(&paths, Duration::from_millis(150), &mut NoopRunner, None)
            .expect("start");
        let err = super::stop_internal_with_runner(&paths, false, &mut FailCleanupRunner)
            .expect_err("stop must fail when cleanup fails");
        std::env::set_var("PATH", old_path);
        assert_eq!(err.code, crate::error::ErrorCode::Command);
        // Core MUST still be alive!
        assert_eq!(status(&paths).expect("status")["running"], true);
        super::stop_internal_with_runner(&paths, false, &mut NoopRunner).expect("cleanup stop");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parse_cpu_list_and_memcg_limit_work() {
        assert_eq!(super::parse_cpu_list("0-3,6"), Some(vec![0, 1, 2, 3, 6]));
        assert_eq!(super::parse_cpu_list("bad"), None);
        assert_eq!(super::parse_memcg_limit("100M"), Some(100 * 1024 * 1024));
        assert_eq!(super::parse_memcg_limit("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(super::parse_memcg_limit("512K"), Some(512 * 1024));
        assert_eq!(super::parse_memcg_limit("invalid"), None);
    }

    #[test]
    fn core_dns_keeps_fake_ip_even_with_lists() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(root.join("settings.json"), r#"{"network_mode":"tproxy"}"#).expect("settings");
        fs::write(root.join("package.list.cfg"), "com.example.app\n").expect("packages");
        let candidate = root.join("candidate.yaml");
        fs::write(&candidate, "dns:\n  enhanced-mode: fake-ip\n").expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("tproxy")).expect("prepare");
        let text = fs::read_to_string(&candidate).expect("read");
        assert!(text.contains("enhanced-mode: fake-ip"), "{text}");
        assert!(!text.contains("sniffer:"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn candidate_uses_find_process_mode_from_settings() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(
            root.join("settings.json"),
            r#"{"network_mode":"mixed","find_process_mode":"always"}"#,
        )
        .expect("settings");
        let candidate = root.join("candidate.yaml");
        fs::write(&candidate, "mode: rule\n").expect("candidate");
        super::prepare_candidate(&paths, &candidate, Some("mixed")).expect("prepare");
        let text = fs::read_to_string(&candidate).expect("read");
        assert!(text.contains("find-process-mode: always"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_tun_candidate_keeps_requested_off_mode() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(
            root.join("settings.json"),
            r#"{"network_mode":"tproxy","find_process_mode":"off"}"#,
        )
        .expect("settings");
        let candidate = root.join("candidate.yaml");
        fs::write(
            &candidate,
            "tun:\n  enable: true\nfind-process-mode: always\n",
        )
        .expect("candidate");
        super::disable_tun_candidate(&paths, &candidate).expect("prepare candidate");
        let text = fs::read_to_string(&candidate).expect("read");
        assert!(text.contains("find-process-mode: off"), "{text}");
        assert!(!text.contains("find-process-mode: always"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn supervisor_decision_respects_intent_manual_and_exhaustion() {
        assert_eq!(
            super::supervisor_action(Some("stopped"), false, Some("applied"), 0),
            super::SupervisorAction::Idle
        );
        assert_eq!(
            super::supervisor_action(Some("running"), true, Some("applied"), 3),
            super::SupervisorAction::Running
        );
        assert_eq!(
            super::supervisor_action(Some("running"), false, Some("manual"), 0),
            super::SupervisorAction::SkipManual
        );
        assert_eq!(
            super::supervisor_action(Some("running"), false, Some("applied"), 5),
            super::SupervisorAction::Exhausted
        );
        assert_eq!(
            super::supervisor_action(Some("running"), false, Some("applied"), 2),
            super::SupervisorAction::Restart
        );
    }

    #[test]
    fn backoff_is_bounded_and_monotonic() {
        assert_eq!(backoff_seconds(0), 1);
        assert_eq!(backoff_seconds(1), 2);
        assert_eq!(backoff_seconds(2), 4);
        assert_eq!(backoff_seconds(3), 8);
        assert_eq!(backoff_seconds(4), 16);
        assert_eq!(backoff_seconds(99), 16);
    }

    #[test]
    fn reload_uses_external_controller() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = "{}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write");
        });
        let root = temp_root();
        fs::write(
            root.join("mihomo/config.yaml"),
            format!("external-controller: {address}\nsecret: test\n"),
        )
        .expect("config");
        let paths = crate::paths::Paths::new(root.clone());
        let value = reload(&paths).expect("reload");
        assert_eq!(value["status"], 200);
        server.join().expect("server");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn start_requires_upgrade_confirmation() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::create_dir_all(root.join("state")).expect("state");
        fs::write(
            root.join("state/upgrade.json"),
            r#"{"required":true,"confirmed":false,"from":"1.4.0","to":"2.0.0"}"#,
        )
        .expect("upgrade");
        let error = start_with_window(&paths, Duration::from_millis(100), &mut NoopRunner, None)
            .expect_err("upgrade gate");
        assert_eq!(error.code, crate::error::ErrorCode::NeedsConfirmation);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn health_check_rejects_unbound_port_and_accepts_listener() {
        use std::net::TcpListener;
        let root = temp_root();
        let config = root.join("mihomo/config.yaml");
        fs::write(&config, "mixed-port: 1\n").expect("config");
        assert!(health_check(&config).is_err());
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let port = listener.local_addr().expect("addr").port();
        fs::write(&config, format!("mixed-port: {port}\n")).expect("config");
        health_check(&config).expect("listening port");
        drop(listener);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_marks_unfinished_verification_without_running_core() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::create_dir_all(root.join("state")).expect("state");
        fs::write(
            root.join("state/service.json"),
            r#"{"txid":"tx-1","phase":"verifying","last_error":null}"#,
        )
        .expect("state");
        let recovered = super::recover(&paths).expect("recover");
        assert_eq!(recovered["phase"], "applied");
        assert_eq!(recovered["core_running"], false);
        assert!(recovered["last_error"].is_string());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_marks_rolling_back_as_manual_and_preparing_as_idle() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::create_dir_all(root.join("state")).expect("state");
        fs::write(
            root.join("state/service.json"),
            r#"{"txid":"tx-1","phase":"rolling_back","last_error":null}"#,
        )
        .expect("state");
        let rolling = super::recover(&paths).expect("recover rolling");
        assert_eq!(rolling["phase"], "manual");
        assert_eq!(rolling["recovered"], true);
        fs::write(
            root.join("state/service.json"),
            r#"{"txid":"tx-2","phase":"preparing","last_error":null}"#,
        )
        .expect("state");
        let preparing = super::recover(&paths).expect("recover preparing");
        assert_eq!(preparing["phase"], "idle");
        assert_eq!(preparing["recovered"], true);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn injected_switch_fault_leaves_recoverable_transaction() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let error = start_with_window(
            &paths,
            Duration::from_millis(100),
            &mut NoopRunner,
            Some("switching"),
        )
        .expect_err("fault");
        assert_eq!(error.code, crate::error::ErrorCode::Internal);
        let state = fs::read_to_string(root.join("state/service.json")).expect("state");
        assert!(state.contains("switching"), "{state}");
        let recovered = super::recover(&paths).expect("recover");
        assert_eq!(recovered["phase"], "applied");
        assert_eq!(recovered["core_running"], false);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn injected_verifying_fault_leaves_recoverable_transaction() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let error = start_with_window(
            &paths,
            Duration::from_millis(100),
            &mut NoopRunner,
            Some("verifying"),
        )
        .expect_err("fault");
        assert_eq!(error.code, crate::error::ErrorCode::Internal);
        let recovered = super::recover(&paths).expect("recover");
        assert_eq!(recovered["phase"], "applied");
        assert_eq!(recovered["core_running"], false);
        assert!(recovered["last_error"].is_string());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_failure_rolls_back_rules_and_stops_core() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(root.join("settings.json"), r#"{"network_mode":"tproxy"}"#).expect("settings");
        let _guard = PATH_LOCK.lock().expect("path lock");
        let fake_bin = root.join("fake-bin");
        fs::create_dir_all(&fake_bin).expect("fake bin");
        for name in ["iptables", "ip6tables", "ipset", "nft"] {
            let path = fake_bin.join(name);
            fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fake command");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake");
        }
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fake_bin.display(), old_path));
        assert!(
            crate::network::detect(&paths).iptables,
            "PATH={:?}",
            std::env::var("PATH")
        );
        let mut runner = FailApplyRunner { failed: false };
        let error = start_with_window(&paths, Duration::from_millis(100), &mut runner, None)
            .expect_err("apply failure");
        std::env::set_var("PATH", old_path);
        assert_eq!(
            error.code,
            crate::error::ErrorCode::Command,
            "{error} detail={:?}",
            error.detail
        );
        assert!(!paths.box_pid.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn supervisor_restart_preserves_auto_mode() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        start_with_window(&paths, Duration::from_millis(100), &mut NoopRunner, None)
            .expect("initial start");
        stop(&paths).expect("stop");
        super::write_control(&paths, "auto", "running").expect("auto control");
        start_for_restart(&paths).expect("restart");
        assert_eq!(super::control(&paths)["mode"], "auto");
        assert_eq!(super::control(&paths)["desired"], "running");
        stop(&paths).expect("final stop");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn injected_rollback_fault_recovers_to_manual() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(root.join("settings.json"), r#"{"network_mode":"tproxy"}"#).expect("settings");
        let _guard = PATH_LOCK.lock().expect("path lock");
        let fake_bin = root.join("fake-bin");
        fs::create_dir_all(&fake_bin).expect("fake bin");
        for name in ["iptables", "ip6tables", "ipset", "nft"] {
            let path = fake_bin.join(name);
            fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fake command");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fake_bin.display(), old_path));
        let mut runner = FailApplyRunner { failed: false };
        let error = start_with_window(
            &paths,
            Duration::from_millis(100),
            &mut runner,
            Some("rolling_back"),
        )
        .expect_err("fault");
        std::env::set_var("PATH", old_path);
        assert_eq!(error.code, crate::error::ErrorCode::Internal);
        let recovered = super::recover(&paths).expect("recover");
        assert_eq!(recovered["phase"], "manual");
        assert_eq!(recovered["core_running"], false);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn start_and_stop_manage_core_and_transaction() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        let started = start_with_window(&paths, Duration::from_millis(400), &mut NoopRunner, None)
            .expect("start");
        assert_eq!(started["status"]["running"], true);
        assert_eq!(super::control(&paths)["mode"], "manual");
        assert_eq!(super::control(&paths)["desired"], "running");
        let running = status(&paths).expect("status");
        assert_eq!(running["running"], true);
        assert!(root.join("state/tx").is_dir());
        let stopped = stop(&paths).expect("stop");
        assert_eq!(stopped["status"]["running"], false);
        assert_eq!(stopped["warnings"].as_array().map(Vec::len), Some(0));
        assert_eq!(super::control(&paths)["mode"], "manual");
        assert_eq!(super::control(&paths)["desired"], "stopped");
        let after = status(&paths).expect("status after");
        assert_eq!(after["running"], false);
        assert_eq!(after["phase"], "applied");
        let _ = fs::remove_dir_all(root);
    }
}
