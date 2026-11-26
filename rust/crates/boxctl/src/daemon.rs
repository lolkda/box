use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::config;
use crate::core;
use crate::doctor;
use crate::download;
use crate::error::{BoxError, ErrorCode, Result};
use crate::events;
use crate::install;
use crate::lists;
use crate::logs;
use crate::network;
use crate::network_rules;
use crate::paths::Paths;
use crate::peer;
use crate::protocol::{Request, Response, MAX_FRAME_BYTES, PROTOCOL_VERSION};
use crate::rule_profile;
use crate::schedule;
use crate::service;
use crate::settings::{ApplyParams, SetParams, SettingsStore};
use crate::sub;
use crate::task;
use crate::uninstall;
use crate::upgrade;
use crate::util;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn shutdown_handler(_signal: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn install_shutdown_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            shutdown_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            shutdown_handler as *const () as libc::sighandler_t,
        );
    }
}

fn acquire_daemon_lock(paths: &Paths) -> Result<File> {
    util::ensure_dir(
        paths
            .daemon_lock
            .parent()
            .ok_or_else(|| BoxError::internal("daemon lock has no parent"))?,
        true,
    )?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&paths.daemon_lock)
        .map_err(|error| BoxError::from(error).with_detail("cannot open daemon lock"))?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        return Err(BoxError::new(
            ErrorCode::Conflict,
            "another boxctl daemon already holds the daemon lock",
        ));
    }
    Ok(file)
}

pub fn run(paths: Paths, socket: PathBuf) -> Result<()> {
    let store = SettingsStore::open(paths.clone())?;
    paths.ensure_run_dir()?;
    let _daemon_lock = acquire_daemon_lock(&paths)?;
    if socket.exists() {
        let metadata = fs::symlink_metadata(&socket).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot stat {}", socket.display()))
        })?;
        if !metadata.file_type().is_socket() {
            return Err(
                BoxError::new(ErrorCode::Io, "socket path exists and is not a socket")
                    .with_detail(socket.display().to_string()),
            );
        }
        if UnixStream::connect(&socket).is_ok() {
            return Err(BoxError::new(
                ErrorCode::Conflict,
                "another boxctl daemon is already listening on this socket",
            )
            .with_detail(socket.display().to_string()));
        }
        fs::remove_file(&socket).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot remove stale {}", socket.display()))
        })?;
    }
    if let Some(parent) = socket.parent() {
        util::ensure_dir(parent, true)?;
    }

    install_shutdown_handlers();
    let listener = UnixListener::bind(&socket).map_err(|error| {
        BoxError::from(error).with_detail(format!("cannot bind {}", socket.display()))
    })?;
    listener.set_nonblocking(true).map_err(BoxError::from)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).map_err(|error| {
        BoxError::from(error).with_detail(format!("cannot chmod {}", socket.display()))
    })?;
    util::atomic_write(
        &paths.pid_file,
        format!("{}\n", std::process::id()).as_bytes(),
        0o600,
    )?;
    util::atomic_write(
        &paths.daemon_pid,
        format!("{}\n", std::process::id()).as_bytes(),
        0o600,
    )?;

    let store = Arc::new(Mutex::new(store));
    if let Err(error) = service::recover(&paths) {
        eprintln!("boxctl service recovery failed: {error}");
    }
    let autostart_paths = paths.clone();
    thread::spawn(move || boot_autostart(autostart_paths));
    let capabilities_paths = paths.clone();
    thread::spawn(move || {
        let _ = network::warm_capabilities(&capabilities_paths);
    });
    let supervisor_paths = paths.clone();
    thread::spawn(move || service::supervisor_loop(supervisor_paths));
    let scheduler_paths = paths.clone();
    thread::spawn(move || scheduler_loop(scheduler_paths));
    let event_paths = paths.clone();
    thread::spawn(move || events::control_loop(event_paths));
    let disable_paths = paths.clone();
    thread::spawn(move || module_disable_loop(disable_paths));
    while !SHUTDOWN.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _address)) => {
                let store = Arc::clone(&store);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &store) {
                        eprintln!("boxctl connection failed: {error}");
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                eprintln!("boxctl accept failed: {error}");
                return Err(BoxError::from(error).with_detail("accept failed"));
            }
        }
    }
    let _ = fs::remove_file(&socket);
    let _ = fs::remove_file(&paths.pid_file);
    let _ = fs::remove_file(&paths.daemon_pid);
    Ok(())
}

fn boot_autostart(paths: Paths) {
    if std::env::var("BOXCTL_BOOT_AUTOSTART").as_deref() != Ok("1") {
        return;
    }
    thread::sleep(std::time::Duration::from_secs(1));
    // Honor Root manager module disable flag (legacy start.sh / box.inotify).
    if paths.module_disable.exists() {
        if service::status(&paths)
            .ok()
            .and_then(|status| status["running"].as_bool())
            .unwrap_or(false)
        {
            let _ = service::stop(&paths);
        }
        return;
    }
    let Ok(store) = SettingsStore::open(paths.clone()) else {
        return;
    };
    let Ok(settings) = store.load() else {
        return;
    };
    if settings.get("boot_auto_start").and_then(Value::as_str) != Some("true") {
        if service::status(&paths)
            .ok()
            .and_then(|status| status["running"].as_bool())
            .unwrap_or(false)
        {
            let _ = service::stop(&paths);
        }
        return;
    }
    let control = service::control(&paths);
    if control["mode"].as_str() == Some("manual") && control["desired"].as_str() == Some("stopped")
    {
        return;
    }
    if upgrade::status(&paths)
        .ok()
        .and_then(|status| status["required"].as_bool())
        .unwrap_or(false)
    {
        return;
    }
    if let Err(error) = service::start(&paths) {
        eprintln!("boxctl boot autostart failed: {error}");
    } else if control["mode"].as_str() == Some("auto") {
        let _ = service::write_control(&paths, "auto", "running");
    }
}

/// Native replacement for legacy `box.inotify`:
/// Watches `/data/adb/modules/box_for_root/disable`. Creating it immediately stops
/// the core & cleans up rules; removing it restores the service if it was disabled-stopped.
fn module_disable_loop(paths: Paths) {
    let mut was_disabled = paths.module_disable.exists();
    let mut stopped_by_disable = false;
    loop {
        thread::sleep(std::time::Duration::from_secs(2));
        if SHUTDOWN.load(Ordering::SeqCst) {
            return;
        }
        let disabled = paths.module_disable.exists();
        if disabled && !was_disabled {
            let running = service::status(&paths)
                .ok()
                .and_then(|s| s["running"].as_bool())
                .unwrap_or(false);
            if running {
                let _ = service::stop(&paths);
                stopped_by_disable = true;
            }
            was_disabled = true;
        } else if !disabled && was_disabled {
            was_disabled = false;
            if stopped_by_disable {
                let _ = service::start(&paths);
                stopped_by_disable = false;
            }
        }
    }
}

fn scheduler_loop(paths: Paths) {
    let mut custom_last_runs: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    loop {
        thread::sleep(std::time::Duration::from_secs(30));
        if SHUTDOWN.load(Ordering::SeqCst) {
            return;
        }
        if paths.module_disable.exists() {
            continue;
        }
        let Ok(store) = SettingsStore::open(paths.clone()) else {
            continue;
        };
        let Ok(settings) = store.load() else {
            continue;
        };
        let settings_value = Value::Object(settings);
        let Ok(scheduler) = schedule::Scheduler::from_settings(&settings_value) else {
            continue;
        };
        let now = schedule::now_seconds();

        if scheduler.enabled && !scheduler.tasks.is_empty() {
            if let Ok(state) = schedule::state(&paths) {
                match schedule::check_due(&scheduler, &state, now, 3600) {
                    schedule::DueCheck::AdvanceBaseline => {
                        // Advance baseline so first enable doesn't fire immediately and
                        // sleeping > 1h never permanently stalls the scheduler.
                        let _ = schedule::mark_run(
                            &paths,
                            state.last_task.as_deref().unwrap_or("baseline"),
                            now,
                        );
                    }
                    schedule::DueCheck::Due => {
                        if task::running_count(&paths).unwrap_or(0) == 0 {
                            let mut last_kind = None;
                            for kind in &scheduler.tasks {
                                if task::running_count(&paths).unwrap_or(0) > 0 {
                                    break;
                                }
                                if task::start(
                                    &paths,
                                    task::StartParams {
                                        kind: kind.clone(),
                                        arg: None,
                                        wait: true,
                                    },
                                )
                                .is_ok()
                                {
                                    last_kind = Some(kind.clone());
                                }
                            }
                            let _ = schedule::mark_run(
                                &paths,
                                last_kind.as_deref().unwrap_or("skipped"),
                                now,
                            );
                        }
                    }
                    schedule::DueCheck::NotDue => {}
                }
            }
        }

        // Also execute legacy `box/crontab.cfg` entries when `run_crontab=true`.
        if scheduler.enabled {
            for entry in schedule::load_crontab_entries(&paths) {
                let last = *custom_last_runs.entry(entry.id.clone()).or_insert(now);
                if let Some(next) = entry.cron.next_after(last) {
                    if next <= now {
                        if now.saturating_sub(next) <= 3600 {
                            schedule::run_crontab_entry(&paths, &entry);
                        }
                        custom_last_runs.insert(entry.id.clone(), now);
                    }
                }
            }
        }
    }
}

fn handle_connection(stream: UnixStream, store: &Arc<Mutex<SettingsStore>>) -> Result<()> {
    let peer_uid = peer::peer_uid(&stream)
        .map_err(|error| BoxError::from(error).with_detail("cannot read SO_PEERCRED"))?;
    if !peer::is_authorized(peer_uid) {
        send_response(
            &stream,
            &Response::error(
                BoxError::permission("socket is root-only")
                    .with_detail(format!("peer uid={peer_uid}")),
                "",
                0,
            ),
        )?;
        return Ok(());
    }

    let mut reader = BufReader::new(stream.try_clone().map_err(BoxError::from)?);
    let line = match read_frame(&mut reader) {
        Ok(Some(line)) => line,
        Ok(None) => return Ok(()),
        Err(error) => {
            send_response(&stream, &Response::error(error, "", 0))?;
            return Ok(());
        }
    };
    let started = Instant::now();
    let request = match Request::parse(&line) {
        Ok(request) => request,
        Err(error) => {
            send_response(&stream, &Response::error(error, "", elapsed_ms(started)))?;
            return Ok(());
        }
    };

    if request.protocol != PROTOCOL_VERSION {
        let error = BoxError::protocol(format!(
            "unsupported protocol {}; expected {PROTOCOL_VERSION}",
            request.protocol
        ));
        send_response(
            &stream,
            &Response::error(error, request.id.clone(), elapsed_ms(started)),
        )?;
        return Ok(());
    }
    if request.client_version.trim().is_empty() {
        send_response(
            &stream,
            &Response::error(
                BoxError::protocol("client_version is required"),
                request.id.clone(),
                elapsed_ms(started),
            ),
        )?;
        return Ok(());
    }

    let response = match dispatch(store, &request) {
        Ok((data, revision)) => {
            Response::success(data, request.id.clone(), elapsed_ms(started), revision)
        }
        Err(error) => Response::error(error, request.id.clone(), elapsed_ms(started)),
    };
    send_response(&stream, &response)
}

fn read_frame(reader: &mut BufReader<UnixStream>) -> Result<Option<String>> {
    let mut line = String::new();
    let mut limited = reader.by_ref().take((MAX_FRAME_BYTES + 1) as u64);
    let read = limited.read_line(&mut line).map_err(BoxError::from)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_FRAME_BYTES || !line.ends_with('\n') {
        return Err(BoxError::protocol(
            "request frame must end with newline and be at most 1 MiB",
        ));
    }
    line.pop();
    if line.ends_with('\r') {
        line.pop();
    }
    if line.trim().is_empty() {
        return Err(BoxError::protocol("empty request frame"));
    }
    Ok(Some(line))
}

fn dispatch(store: &Arc<Mutex<SettingsStore>>, request: &Request) -> Result<(Value, Option<u64>)> {
    match request.method.as_str() {
        "settings.get" => {
            ensure_empty_params(request)?;
            let store = lock_store(store)?;
            let data = store.get()?;
            Ok((data, Some(store.revision())))
        }
        "settings.apply" => {
            let params = parse_params::<ApplyParams>(request)?;
            let mut store = lock_store(store)?;
            let outcome = store.apply(params)?;
            Ok((
                json!({ "changed": outcome.changed }),
                Some(outcome.revision),
            ))
        }
        "settings.set" => {
            let params = parse_params::<SetParams>(request)?;
            let mut store = lock_store(store)?;
            let outcome = store.set(params)?;
            Ok((
                json!({ "changed": outcome.changed }),
                Some(outcome.revision),
            ))
        }
        "config.get" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = config::get(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "config.apply" => {
            let params = parse_params::<config::ApplyParams>(request)?;
            let paths = paths_for(store)?;
            let data = config::apply(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "network_rules.get" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = network_rules::get(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((
                serde_json::to_value(data).map_err(|error| {
                    BoxError::internal("cannot serialize network rules")
                        .with_detail(error.to_string())
                })?,
                Some(revision),
            ))
        }
        "network_rules.preview" => {
            let params = parse_params::<network_rules::Mutation>(request)?;
            let paths = paths_for(store)?;
            let mut data = network_rules::preview(&paths, params)?;
            if let Ok(plan) = service::preview_network_plan(&paths, &data.document) {
                data.plan = Some(plan);
            }
            let revision = lock_store(store)?.revision();
            Ok((
                serde_json::to_value(data).map_err(|error| {
                    BoxError::internal("cannot serialize network preview")
                        .with_detail(error.to_string())
                })?,
                Some(revision),
            ))
        }
        "network_rules.apply" => {
            let params = parse_params::<network_rules::Mutation>(request)?;
            let paths = paths_for(store)?;
            let old = network_rules::load_for_plan(&paths)?;
            let data = network_rules::apply(&paths, params)?;
            let running = match service::status(&paths) {
                Ok(status) => status["running"].as_bool().unwrap_or(false),
                Err(error) => {
                    let _ = network_rules::save_document(&paths, &old);
                    return Err(error);
                }
            };
            if running {
                if let Err(error) = service::apply_network_rules(&paths, Some(&old)) {
                    let _ = network_rules::save_document(&paths, &old);
                    return Err(error);
                }
            }
            let mut value = serde_json::to_value(data).map_err(|error| {
                BoxError::internal("cannot serialize network rules").with_detail(error.to_string())
            })?;
            if let Some(map) = value.as_object_mut() {
                map.insert("applied".to_string(), Value::Bool(running));
            }
            let revision = bump_revision(store)?;
            Ok((value, Some(revision)))
        }
        "network_rules.reset" => {
            let params = parse_params::<ResetNetworkRulesParams>(request)?;
            let paths = paths_for(store)?;
            let old = network_rules::load_for_plan(&paths)?;
            let data = network_rules::reset_defaults(&paths, params.all)?;
            let running = match service::status(&paths) {
                Ok(status) => status["running"].as_bool().unwrap_or(false),
                Err(error) => {
                    let _ = network_rules::save_document(&paths, &old);
                    return Err(error);
                }
            };
            if running {
                if let Err(error) = service::apply_network_rules(&paths, Some(&old)) {
                    let _ = network_rules::save_document(&paths, &old);
                    return Err(error);
                }
            }
            let mut value = serde_json::to_value(data).map_err(|error| {
                BoxError::internal("cannot serialize network rules").with_detail(error.to_string())
            })?;
            if let Some(map) = value.as_object_mut() {
                map.insert("applied".to_string(), Value::Bool(running));
            }
            let revision = bump_revision(store)?;
            Ok((value, Some(revision)))
        }
        "config.delete" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = config::delete(&paths)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "core.get" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let mut data = core::get(&paths)?;
            if let Some(structured) = data.get_mut("structured").and_then(Value::as_object_mut) {
                structured.insert(
                    "find-process-mode".to_string(),
                    json!(service::find_process_mode(&paths)),
                );
            }
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "core.apply" => {
            let params = parse_params::<core::ApplyParams>(request)?;
            let requested_fpm = params
                .structured
                .as_ref()
                .and_then(|s| s.find_process_mode.clone());
            let paths = paths_for(store)?;
            let data = core::apply(&paths, params)?;
            // Keep settings.json `find_process_mode` in sync when changed via core.apply
            // so core.get never reads back a stale value.
            if let Some(fpm) = requested_fpm {
                if matches!(fpm.as_str(), "always" | "strict" | "off") {
                    let mut store = lock_store(store)?;
                    let _ = store.set(crate::settings::SetParams {
                        key: "find_process_mode".to_string(),
                        value: fpm,
                        if_revision: None,
                    });
                }
            }
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "lists.get" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = lists::get(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "lists.apply" => {
            let params = parse_params::<lists::ApplyParams>(request)?;
            let paths = paths_for(store)?;
            let data = lists::apply(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "sub.update" => {
            let params = parse_params::<sub::UpdateParams>(request)?;
            let paths = paths_for(store)?;
            let policy = download_policy(store)?;
            let keep_last_good = {
                let store = lock_store(store)?;
                let settings = store.load()?;
                settings
                    .get("subscription_keep_last_good")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == "true")
            };
            let data = sub::update(&paths, params, &policy, keep_last_good)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "sub.list" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let (ua, _) = subscription_context(store)?;
            let data = sub::list(&paths, &ua)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "sub.get" => {
            let params = parse_params::<sub::NameParams>(request)?;
            let paths = paths_for(store)?;
            let (ua, _) = subscription_context(store)?;
            let data = sub::get(&paths, params, &ua)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "sub.add" => {
            let params = parse_params::<sub::AddParams>(request)?;
            let paths = paths_for(store)?;
            let (ua, _) = subscription_context(store)?;
            let data = sub::add(&paths, params, &ua)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "sub.set" => {
            let params = parse_params::<sub::SetParams>(request)?;
            let paths = paths_for(store)?;
            let (ua, _) = subscription_context(store)?;
            let data = sub::set(&paths, params, &ua)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "sub.rm" => {
            let params = parse_params::<sub::NameParams>(request)?;
            let paths = paths_for(store)?;
            let data = sub::remove(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "sub.enable" | "sub.disable" => {
            let enabled = request.method == "sub.enable";
            let params = parse_params::<sub::NameParams>(request)?;
            let paths = paths_for(store)?;
            let (ua, auto_modify) = subscription_context(store)?;
            let data = sub::set_enabled(&paths, params, enabled, auto_modify, &ua)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "logs.get" => {
            let params = parse_params::<logs::GetParams>(request)?;
            let paths = paths_for(store)?;
            let data = logs::get(&paths, params)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "logs.clear" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = logs::clear(&paths)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "reset.subscriptions" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = sub::reset(&paths)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.list" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = rule_profile::list(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "rule_profile.get" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = rule_profile::get(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "rule_profile.add" => {
            let params = parse_params::<rule_profile::AddParams>(request)?;
            let paths = paths_for(store)?;
            let data = rule_profile::add(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.edit" => {
            let params = parse_params::<rule_profile::EditParams>(request)?;
            let paths = paths_for(store)?;
            let data = rule_profile::edit(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.remove" => {
            let params = parse_params::<rule_profile::IdParams>(request)?;
            let paths = paths_for(store)?;
            let data = rule_profile::remove(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.deactivate" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = rule_profile::deactivate(&paths)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.url" => {
            let params = parse_params::<rule_profile::UrlParams>(request)?;
            let store = lock_store(store)?;
            let data = rule_profile::url(store.paths(), params)?;
            Ok((data, Some(store.revision())))
        }
        "rule_profile.set" => {
            let params = parse_params::<rule_profile::SetUrlParams>(request)?;
            let data = {
                let store = lock_store(store)?;
                rule_profile::set(store.paths(), params)?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.clear" => {
            ensure_empty_params(request)?;
            let data = {
                let store = lock_store(store)?;
                rule_profile::clear(store.paths())?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.apply" => {
            let params = parse_params::<rule_profile::UseParams>(request)?;
            let paths = paths_for(store)?;
            let mut policy = download_policy(store)?;
            policy.user_agent = "mihomo".to_string();
            let data = rule_profile::apply_with_policy(&paths, params, Some(&policy))?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.fetch" => {
            let params = parse_params::<rule_profile::FetchParams>(request)?;
            let paths = paths_for(store)?;
            let mut policy = download_policy(store)?;
            policy.user_agent = "mihomo".to_string();
            let data = rule_profile::fetch(&paths, params, &policy)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "rule_profile.use" => {
            let params = parse_params::<rule_profile::UseParams>(request)?;
            let paths = paths_for(store)?;
            let mut policy = download_policy(store)?;
            policy.user_agent = "mihomo".to_string();
            let data = rule_profile::use_pending_with_policy(&paths, params, Some(&policy))?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "task.start" => {
            let params = parse_params::<task::StartParams>(request)?;
            let paths = paths_for(store)?;
            let data = task::start(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "task.list" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = task::list(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "task.status" => {
            let params = parse_params::<task::IdParams>(request)?;
            let paths = paths_for(store)?;
            let data = task::status(&paths, params)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "task.logs" => {
            let params = parse_params::<task::LogsParams>(request)?;
            let paths = paths_for(store)?;
            let data = task::logs(&paths, params)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "task.cancel" => {
            let params = parse_params::<task::IdParams>(request)?;
            let paths = paths_for(store)?;
            let data = task::cancel(&paths, params)?;
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "service.start" | "service.stop" | "service.restart" | "service.reload" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = match request.method.as_str() {
                "service.start" => service::start(&paths)?,
                "service.stop" => service::stop(&paths)?,
                "service.restart" => service::restart(&paths)?,
                _ => service::reload(&paths)?,
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "internal.control_auto" | "internal.control_manual" => {
            ensure_empty_params(request)?;
            let mode = if request.method == "internal.control_auto" {
                "auto"
            } else {
                "manual"
            };
            let data = {
                let store = lock_store(store)?;
                let running = service::status(store.paths())
                    .ok()
                    .and_then(|status| status["running"].as_bool())
                    .unwrap_or(false);
                service::write_control(
                    store.paths(),
                    mode,
                    if running { "running" } else { "stopped" },
                )?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "internal.event_reset" => {
            ensure_empty_params(request)?;
            let data = {
                let store = lock_store(store)?;
                events::reset_control(store.paths())?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "internal.upgrade_prepare" => {
            ensure_empty_params(request)?;
            let data = {
                let store = lock_store(store)?;
                install::prepare_upgrade(store.paths())?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "uninstall.preflight" => {
            ensure_empty_params(request)?;
            let store = lock_store(store)?;
            let data = uninstall::preflight(store.paths())?;
            Ok((data, Some(store.revision())))
        }
        "uninstall.apply" => {
            ensure_empty_params(request)?;
            let data = {
                let store = lock_store(store)?;
                uninstall::apply(store.paths())?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "uninstall.force" => {
            let params = parse_params::<uninstall::ForceParams>(request)?;
            let data = {
                let store = lock_store(store)?;
                uninstall::force(store.paths(), params)?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "purge.plan" => {
            ensure_empty_params(request)?;
            let store = lock_store(store)?;
            let data = uninstall::plan(store.paths())?;
            Ok((data, Some(store.revision())))
        }
        "purge.apply" => {
            let params = parse_params::<uninstall::PurgeApplyParams>(request)?;
            let data = {
                let store = lock_store(store)?;
                uninstall::apply_purge(store.paths(), params)?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "upgrade.status" => {
            ensure_empty_params(request)?;
            let store = lock_store(store)?;
            let data = upgrade::status(store.paths())?;
            Ok((data, Some(store.revision())))
        }
        "upgrade.confirm" => {
            let params = parse_params::<upgrade::ConfirmParams>(request)?;
            let data = {
                let store = lock_store(store)?;
                upgrade::confirm(store.paths(), params)?
            };
            let revision = bump_revision(store)?;
            Ok((data, Some(revision)))
        }
        "doctor.check" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = doctor::check(&paths, true)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "config.check" => {
            ensure_empty_params(request)?;
            let paths = paths_for(store)?;
            let data = service::check_config(&paths)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "network.bond0" => {
            ensure_empty_params(request)?;
            let data = network::apply_bond(0)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "network.bond1" => {
            ensure_empty_params(request)?;
            let data = network::apply_bond(1)?;
            let revision = lock_store(store)?.revision();
            Ok((data, Some(revision)))
        }
        "status.get" => {
            ensure_empty_params(request)?;
            let store = lock_store(store)?;
            let revision = store.revision();
            let mut warnings = Vec::new();
            let settings = store.load();
            let mode = match &settings {
                Ok(settings) => settings
                    .get("network_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                Err(error) => {
                    warnings.push(json!({"code": error.code.as_str(), "message": error.message}));
                    "unknown".to_string()
                }
            };
            let capabilities = network::cached_capabilities(store.paths());
            let network_plan = capabilities.and_then(|_| {
                service::load_network_plan(store.paths()).ok().flatten()
            });
            let schedule = settings.as_ref().ok().and_then(|settings| {
                schedule::status(
                    store.paths(),
                    &Value::Object(settings.clone()),
                    schedule::now_seconds(),
                )
                .ok()
            });
            let network_event = settings
                .as_ref()
                .ok()
                .map(|settings| events::snapshot(&Value::Object(settings.clone())));
            let event_control = events::control_status(store.paths());
            let control = service::control(store.paths());
            let subscriptions = sub::count(store.paths()).unwrap_or(0);
            let active_tasks = task::running_count(store.paths()).unwrap_or(0);
            let service = service::status(store.paths())?;
            let data = json!({
                "daemon": true,
                "degraded": false,
                "running": service["running"],
                "pid": service["pid"],
                "core": "mihomo",
                "mode": mode,
                "config": store.paths().config.display().to_string(),
                "config_exists": store.paths().config.exists(),
                "subscriptions": subscriptions,
                "active_tasks": active_tasks,
                "revision": revision,
                "txid": service["txid"],
                "phase": service["phase"],
                "last_error": service["last_error"],
                "upgrade": upgrade::status(store.paths())?,
                "capabilities": capabilities.unwrap_or_default().json(),
                "network_plan": network_plan.map(|plan| plan.json()),
                "schedule": schedule,
                "network_event": network_event,
                "event_control": event_control,
                "control": control,
                "warnings": warnings
            });
            Ok((data, Some(revision)))
        }
        "version.get" => {
            ensure_empty_params(request)?;
            let store = lock_store(store)?;
            let data = json!({
                "server_version": env!("CARGO_PKG_VERSION"),
                "contract_version": crate::protocol::CONTRACT_VERSION,
                "protocol": PROTOCOL_VERSION,
                "commit": option_env!("BOXCTL_COMMIT").unwrap_or("unknown")
            });
            Ok((data, Some(store.revision())))
        }
        other => Err(BoxError::new(
            ErrorCode::NotFound,
            format!("unknown method: {other}"),
        )),
    }
}

fn lock_store(store: &Arc<Mutex<SettingsStore>>) -> Result<MutexGuard<'_, SettingsStore>> {
    store
        .lock()
        .map_err(|_| BoxError::internal("settings store lock poisoned"))
}

fn download_policy(store: &Arc<Mutex<SettingsStore>>) -> Result<download::Policy> {
    let store = lock_store(store)?;
    download::policy_from_settings(&store)
}

fn subscription_context(store: &Arc<Mutex<SettingsStore>>) -> Result<(String, bool)> {
    let store = lock_store(store)?;
    let settings = store.load()?;
    let ua = settings
        .get("subscription_user_agent")
        .and_then(Value::as_str)
        .unwrap_or("clash-verge/v2.4.5")
        .to_string();
    let auto_modify = settings
        .get("auto_modify_config")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    Ok((ua, auto_modify))
}

fn paths_for(store: &Arc<Mutex<SettingsStore>>) -> Result<Paths> {
    Ok(lock_store(store)?.paths().clone())
}

fn bump_revision(store: &Arc<Mutex<SettingsStore>>) -> Result<u64> {
    lock_store(store)?.bump_revision()
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetNetworkRulesParams {
    #[serde(default)]
    all: bool,
}

fn parse_params<T: DeserializeOwned>(request: &Request) -> Result<T> {
    let value = request
        .params
        .clone()
        .unwrap_or_else(|| Value::Object(Map::new()));
    serde_json::from_value(value).map_err(|error| {
        BoxError::validation("invalid request params").with_detail(error.to_string())
    })
}

fn ensure_empty_params(request: &Request) -> Result<()> {
    match &request.params {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Object(map)) if map.is_empty() => Ok(()),
        Some(_) => Err(BoxError::validation("this method takes no params")),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn send_response(mut stream: &UnixStream, response: &Response) -> Result<()> {
    let encoded = response.encode()?;
    stream
        .write_all(encoded.as_bytes())
        .map_err(BoxError::from)?;
    stream.flush().map_err(BoxError::from)
}
