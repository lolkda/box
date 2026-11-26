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
    set_phase(paths, &tx, "switching", None)?;
    if fault == Some("switching") {
        return Err(BoxError::internal("injected fault after switching"));
    }
    let mut child = spawn_core(paths, &tx.candidate).inspect_err(|error| {
        let _ = fail_manual(paths, &tx, &error.message);
    })?;
    let pid = child.id();
    write_pid(paths, pid)?;
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
        if plan.rules.is_empty() && plan.requested_mode.as_str() != "tun" {
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
    set_intent(paths, "running")?;
    let _ = write_control(paths, "manual", "running")?;
    let _ = util::remove_file_if_exists(&paths.event_stop);
    let warnings = plan
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
    stop_internal(paths, true)
}

fn stop_internal(paths: &Paths, interrupt_tasks: bool) -> Result<Value> {
    let _ = util::remove_file_if_exists(&paths.event_stop);
    set_intent(paths, "stopped")?;
    let _ = write_control(paths, "manual", "stopped")?;
    if interrupt_tasks {
        let _ = crate::task::interrupt_all(paths);
    }
    let tx = begin_tx(paths, "stop")?;
    set_phase(paths, &tx, "switching", None)?;
    let rules_cleaned = {
        let mut runner = network::SystemRunner;
        cleanup_all_network(paths, &mut runner)?
    };
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
    let config = fs::read_to_string(&paths.config)
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
    let response = request
        .send("{\"path\":\"\",\"payload\":\"\"}")
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
    Ok(json!({
        "action": "reload",
        "status": response.status().as_u16()
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
    let settings = Value::Object(store.load()?);
    let caps = network::detect(paths);
    let cleaned =
        network::cleanup_all(&settings, caps, dns_listen_port(paths), runner).unwrap_or(false);
    Ok(network::cleanup_stale(runner) || cleaned)
}

fn load_network_plan(paths: &Paths) -> Result<Option<network::Plan>> {
    if !paths.settings.is_file() {
        return Ok(None);
    }
    let store = SettingsStore::open(paths.clone())?;
    let mut settings = Value::Object(store.load()?);
    settings["__proxy_uids"] = Value::Array(
        proxy_uids(paths, &settings)
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
    let rules = network_rules::load_for_plan(paths)?;
    build_network_plan(paths, &settings, &rules)
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
    settings["__proxy_uids"] = Value::Array(
        proxy_uids(paths, &settings)
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
    settings["__proxy_uids"] = Value::Array(
        proxy_uids(paths, &settings)
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
    for entry in entries.lines() {
        let (prefix, package) = entry.split_once(':').unwrap_or(("0", entry));
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
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn dns_listen_port(paths: &Paths) -> Option<u16> {
    let text = fs::read_to_string(&paths.config).ok()?;
    let config: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let listen = config
        .get("dns")
        .and_then(|dns| dns.get("listen"))
        .and_then(serde_yaml::Value::as_str)?;
    listen.rsplit(':').next()?.trim().parse().ok()
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
    [&paths.package_list, &paths.gid_list, &paths.ap_list]
        .into_iter()
        .any(|path| {
            fs::read_to_string(path)
                .map(|text| {
                    text.lines().any(|line| {
                        let line = line.trim();
                        !line.is_empty() && !line.starts_with('#')
                    })
                })
                .unwrap_or(false)
        })
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
    let existed = mapping.contains_key(&dns_key);
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
    if should_switch {
        dns.insert(
            yaml_key("enhanced-mode"),
            serde_yaml::Value::String("redir-host".to_string()),
        );
        let sniffer_key = yaml_key("sniffer");
        let mut sniffer = dns
            .get(&sniffer_key)
            .and_then(yaml_mapping)
            .cloned()
            .unwrap_or_default();
        sniffer.insert(yaml_key("enable"), serde_yaml::Value::Bool(true));
        dns.insert(sniffer_key, serde_yaml::Value::Mapping(sniffer));
    }
    let changed = !existed || before != dns;
    if changed {
        mapping.insert(dns_key, serde_yaml::Value::Mapping(dns));
    }
    changed
}

pub fn find_process_mode(paths: &Paths) -> String {
    SettingsStore::open(paths.clone())
        .ok()
        .and_then(|store| store.load().ok())
        .and_then(|settings| settings.get("find_process_mode").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| matches!(value.as_str(), "always" | "strict" | "off"))
        .unwrap_or_else(|| "strict".to_string())
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
        command.pre_exec(|| {
            // Do not leak daemon control sockets or event fds into the Go core.
            for fd in 3..256 {
                libc::close(fd);
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|error| BoxError::from(error).with_detail("cannot spawn core"))
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
    fn mixed_candidate_keeps_tun_and_uses_strict_process_mode() {
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
        assert!(prepared.contains("find-process-mode: strict"), "{prepared}");
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
        assert!(prepared.contains("find-process-mode: strict"), "{prepared}");
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
    fn non_core_dns_switches_to_redir_host_with_sniffer() {
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
        assert!(text.contains("enhanced-mode: redir-host"), "{text}");
        assert!(text.contains("sniffer:"), "{text}");
        assert!(text.contains("enable: true"), "{text}");
        let _ = fs::remove_dir_all(root);
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
