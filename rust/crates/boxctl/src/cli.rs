use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::daemon;
use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::protocol::{Request, Response, MAX_FRAME_BYTES, PROTOCOL_VERSION};
use crate::task;
use crate::util;

#[derive(Debug)]
struct Args {
    json: bool,
    socket: Option<PathBuf>,
    root: Option<PathBuf>,
    timeout: Duration,
    if_revision: Option<u64>,
    stdin: bool,
    reload: bool,
    wait: bool,
    positional: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyFile {
    settings: Map<String, Value>,
    #[serde(default)]
    githubtoken: Option<String>,
    #[serde(default)]
    if_revision: Option<u64>,
}

pub fn run(raw: Vec<String>) -> i32 {
    let args = match parse_args(raw) {
        Ok(args) => args,
        Err(error) => return print_error(&error, false),
    };
    let command = args.positional.first().map(String::as_str).unwrap_or("");
    match command {
        "daemon" => run_daemon(&args),
        "settings" => run_settings(&args),
        "config" => run_config(&args),
        "core" => run_core(&args),
        "lists" => run_lists(&args),
        "sub" => run_sub(&args),
        "rule-profile" => run_rule_profile(&args),
        "network-rules" => run_network_rules(&args),
        "task" => run_task(&args),
        "task-run" => run_task_runner(&args),
        "service" => run_service(&args),
        "upgrade" => run_upgrade(&args),
        "uninstall" => run_uninstall(&args),
        "purge" => run_purge(&args),
        "internal" => run_internal(&args),
        "logs" => run_logs(&args),
        "reset" => run_reset(&args),
        "status" => run_status(&args),
        "doctor" => run_doctor(&args),
        "check" => run_check(&args),
        "bond0" => run_bond(&args, 0),
        "bond1" => run_bond(&args, 1),
        "version" => run_version(&args),
        "" => {
            let error = BoxError::usage(
                "usage: boxctl [--json] [--socket PATH] <settings|config|core|lists|sub|rule-profile|network-rules|task|logs|reset|status|doctor|version|daemon>",
            );
            print_error(&error, args.json)
        }
        other => {
            let error = BoxError::usage(format!("unknown command: {other}"));
            print_error(&error, args.json)
        }
    }
}

fn run_daemon(args: &Args) -> i32 {
    if args.positional.len() != 1 {
        return print_error(
            &BoxError::usage("daemon takes no positional arguments"),
            args.json,
        );
    }
    let paths = Paths::resolve(args.root.clone());
    let socket = paths.socket_for(args.socket.as_deref());
    match daemon::run(paths, socket) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("boxctl daemon: {}", error);
            error.code.exit_code()
        }
    }
}

fn run_settings(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("settings requires get, apply, or set"),
            args.json,
        );
    };
    match subcommand {
        "get" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("settings get takes no arguments"),
                    args.json,
                );
            }
            let socket = socket_path(args);
            match send_request(&socket, "settings.get", json!({}), args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "apply" => apply_settings(args),
        "set" => set_setting(args),
        other => print_error(
            &BoxError::usage(format!("unknown settings command: {other}")),
            args.json,
        ),
    }
}

fn apply_settings(args: &Args) -> i32 {
    let file_arg = args.positional.get(2);
    if args.stdin && file_arg.is_some() {
        return print_error(
            &BoxError::usage("settings apply accepts either a file or --stdin, not both"),
            args.json,
        );
    }
    if !args.stdin && file_arg.is_none() {
        return print_error(
            &BoxError::usage("settings apply requires a JSON file or --stdin"),
            args.json,
        );
    }
    if args.positional.len() > 3 {
        return print_error(
            &BoxError::usage("settings apply takes at most one file"),
            args.json,
        );
    }

    let source = if args.stdin {
        let mut source = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut source) {
            return print_error(&BoxError::from(error), args.json);
        }
        source
    } else {
        let Some(path) = file_arg else {
            return print_error(&BoxError::usage("missing settings file"), args.json);
        };
        match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                return print_error(
                    &BoxError::from(error).with_detail(format!("cannot read {path}")),
                    args.json,
                );
            }
        }
    };
    if source.trim().is_empty() {
        return print_error(&BoxError::usage("settings apply input is empty"), args.json);
    }
    let value: Value = match serde_json::from_str(&source) {
        Ok(value) => value,
        Err(error) => {
            return print_error(
                &BoxError::validation("settings apply input is not valid JSON")
                    .with_detail(error.to_string()),
                args.json,
            );
        }
    };
    let file: ApplyFile = match serde_json::from_value(value) {
        Ok(file) => file,
        Err(error) => {
            return print_error(
                &BoxError::validation(
                    "settings apply expects {settings, githubtoken?, if_revision?}",
                )
                .with_detail(error.to_string()),
                args.json,
            );
        }
    };
    let mut settings = file.settings;
    // `settings get` exposes token state under `sensitive`; it is not a setting key.
    settings.remove("sensitive");
    let mut params = Map::new();
    params.insert("settings".to_string(), Value::Object(settings));
    if let Some(token) = file.githubtoken {
        params.insert("githubtoken".to_string(), Value::String(token));
    }
    if let Some(revision) = args.if_revision.or(file.if_revision) {
        params.insert("if_revision".to_string(), Value::Number(revision.into()));
    }
    let socket = socket_path(args);
    match send_request(
        &socket,
        "settings.apply",
        Value::Object(params),
        args.timeout,
    ) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn set_setting(args: &Args) -> i32 {
    if args.positional.len() != 4 {
        return print_error(
            &BoxError::usage("settings set requires <key> <value>"),
            args.json,
        );
    }
    let key = args.positional[2].clone();
    let value = args.positional[3].clone();
    let mut params = Map::new();
    params.insert("key".to_string(), Value::String(key));
    params.insert("value".to_string(), Value::String(value));
    if let Some(revision) = args.if_revision {
        params.insert("if_revision".to_string(), Value::Number(revision.into()));
    }
    let socket = socket_path(args);
    match send_request(&socket, "settings.set", Value::Object(params), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_config(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("config requires get, apply, or delete"),
            args.json,
        );
    };
    match subcommand {
        "get" => {
            if args.positional.len() != 2 {
                return print_error(&BoxError::usage("config get takes no arguments"), args.json);
            }
            let socket = socket_path(args);
            match send_request(&socket, "config.get", json!({}), args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "apply" => {
            let params = match read_json_input(args) {
                Ok(params) => params,
                Err(error) => return print_error(&error, args.json),
            };
            let socket = socket_path(args);
            match send_request(&socket, "config.apply", params, args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "delete" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("config delete takes no arguments"),
                    args.json,
                );
            }
            let socket = socket_path(args);
            match send_request(&socket, "config.delete", json!({}), args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        other => print_error(
            &BoxError::usage(format!("unknown config command: {other}")),
            args.json,
        ),
    }
}

fn run_core(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(&BoxError::usage("core requires get or apply"), args.json);
    };
    match subcommand {
        "get" => {
            if args.positional.len() != 2 {
                return print_error(&BoxError::usage("core get takes no arguments"), args.json);
            }
            let socket = socket_path(args);
            match send_request(&socket, "core.get", json!({}), args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "apply" => {
            let params = match read_json_input(args) {
                Ok(params) => params,
                Err(error) => return print_error(&error, args.json),
            };
            let socket = socket_path(args);
            match send_request(&socket, "core.apply", params, args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        other => print_error(
            &BoxError::usage(format!("unknown core command: {other}")),
            args.json,
        ),
    }
}

fn run_lists(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(&BoxError::usage("lists requires get or apply"), args.json);
    };
    match subcommand {
        "get" => {
            if args.positional.len() != 2 {
                return print_error(&BoxError::usage("lists get takes no arguments"), args.json);
            }
            let socket = socket_path(args);
            match send_request(&socket, "lists.get", json!({}), args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "apply" => {
            let params = match read_json_input(args) {
                Ok(params) => params,
                Err(error) => return print_error(&error, args.json),
            };
            let socket = socket_path(args);
            match send_request(&socket, "lists.apply", params, args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        other => print_error(
            &BoxError::usage(format!("unknown lists command: {other}")),
            args.json,
        ),
    }
}

fn run_sub(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("sub requires list/get/add/set/rm/enable/disable/update"),
            args.json,
        );
    };
    let socket = socket_path(args);
    let (method, params, expected) = match subcommand {
        "list" => ("sub.list", json!({}), &[2usize][..]),
        "get" => {
            if args.positional.len() != 3 {
                return print_error(&BoxError::usage("sub get requires <name>"), args.json);
            }
            (
                "sub.get",
                json!({"name": args.positional[2]}),
                &[3usize][..],
            )
        }
        "add" => {
            if !(3..=5).contains(&args.positional.len()) {
                return print_error(
                    &BoxError::usage("sub add requires <url> [name] [user-agent]"),
                    args.json,
                );
            }
            let mut params = Map::new();
            params.insert("url".to_string(), Value::String(args.positional[2].clone()));
            if let Some(name) = args.positional.get(3) {
                params.insert("name".to_string(), Value::String(name.clone()));
            }
            if let Some(ua) = args.positional.get(4) {
                params.insert("user_agent".to_string(), Value::String(ua.clone()));
            }
            ("sub.add", Value::Object(params), &[3usize, 4, 5][..])
        }
        "set" => {
            if !(5..=6).contains(&args.positional.len()) {
                return print_error(
                    &BoxError::usage("sub set requires <old-name> <new-name> <url> [user-agent]"),
                    args.json,
                );
            }
            let mut params = Map::new();
            params.insert(
                "old_name".to_string(),
                Value::String(args.positional[2].clone()),
            );
            params.insert(
                "new_name".to_string(),
                Value::String(args.positional[3].clone()),
            );
            params.insert("url".to_string(), Value::String(args.positional[4].clone()));
            if let Some(ua) = args.positional.get(5) {
                params.insert("user_agent".to_string(), Value::String(ua.clone()));
            }
            ("sub.set", Value::Object(params), &[5usize, 6][..])
        }
        "update" => {
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage("sub update [name] takes at most one name"),
                    args.json,
                );
            }
            let params = match args.positional.get(2) {
                Some(name) => json!({"name": name}),
                None => json!({}),
            };
            ("sub.update", params, &[2usize, 3][..])
        }
        "rm" | "enable" | "disable" => {
            if args.positional.len() != 3 {
                return print_error(
                    &BoxError::usage(format!("sub {subcommand} requires <name>")),
                    args.json,
                );
            }
            let method = match subcommand {
                "rm" => "sub.rm",
                "enable" => "sub.enable",
                _ => "sub.disable",
            };
            (method, json!({"name": args.positional[2]}), &[3usize][..])
        }
        other => {
            return print_error(
                &BoxError::usage(format!("unknown sub command: {other}")),
                args.json,
            );
        }
    };
    if !expected.contains(&args.positional.len()) {
        return print_error(&BoxError::usage("invalid sub argument count"), args.json);
    }

    match send_request(&socket, method, params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_network_rules(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("network-rules requires get/preview/apply/reset"),
            args.json,
        );
    };
    let socket = socket_path(args);
    match subcommand {
        "get" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("network-rules get takes no arguments"),
                    args.json,
                );
            }
            match send_request(&socket, "network_rules.get", json!({}), args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "preview" | "apply" => {
            let file_arg = args.positional.get(2);
            if args.stdin && file_arg.is_some() {
                return print_error(
                    &BoxError::usage("network-rules mutation accepts either a file or --stdin"),
                    args.json,
                );
            }
            if !args.stdin && file_arg.is_none() {
                return print_error(
                    &BoxError::usage("network-rules mutation requires a JSON file or --stdin"),
                    args.json,
                );
            }
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage("network-rules mutation takes at most one file"),
                    args.json,
                );
            }
            let source = if args.stdin {
                let mut source = String::new();
                if let Err(error) = io::stdin().read_to_string(&mut source) {
                    return print_error(&BoxError::from(error), args.json);
                }
                source
            } else {
                let path = file_arg.expect("file");
                match fs::read_to_string(path) {
                    Ok(source) => source,
                    Err(error) => {
                        return print_error(
                            &BoxError::from(error).with_detail(format!("cannot read {path}")),
                            args.json,
                        );
                    }
                }
            };
            if source.trim().is_empty() {
                return print_error(&BoxError::usage("network-rules input is empty"), args.json);
            }
            let value: Value = match serde_json::from_str(&source) {
                Ok(value) => value,
                Err(error) => {
                    return print_error(
                        &BoxError::validation("network-rules input is not valid JSON")
                            .with_detail(error.to_string()),
                        args.json,
                    );
                }
            };
            let method = if subcommand == "preview" {
                "network_rules.preview"
            } else {
                "network_rules.apply"
            };
            match send_request(&socket, method, value, args.timeout) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        "reset" | "clear" => {
            let all =
                subcommand == "clear" || args.positional.get(2).is_some_and(|value| value == "all");
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage("network-rules reset takes at most 'all'"),
                    args.json,
                );
            }
            match send_request(
                &socket,
                "network_rules.reset",
                json!({"all": all}),
                args.timeout,
            ) {
                Ok(response) => finish(response, args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        other => print_error(
            &BoxError::usage(format!("unknown network-rules command: {other}")),
            args.json,
        ),
    }
}

fn run_rule_profile(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("rule-profile requires list/get/add/edit/remove/deactivate/url/set/clear/fetch/update/use/apply"),
            args.json,
        );
    };
    let socket = socket_path(args);
    let (method, params) = match subcommand {
        "list" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("rule-profile list takes no arguments"),
                    args.json,
                );
            }
            ("rule_profile.list", json!({}))
        }
        "get" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("rule-profile get takes no arguments"),
                    args.json,
                );
            }
            ("rule_profile.get", json!({}))
        }
        "add" => {
            if args.positional.len() != 4 {
                return print_error(
                    &BoxError::usage("rule-profile add requires <name> <url>"),
                    args.json,
                );
            }
            (
                "rule_profile.add",
                json!({"name": args.positional[2], "url": args.positional[3]}),
            )
        }
        "edit" => {
            if args.positional.len() != 5 {
                return print_error(
                    &BoxError::usage("rule-profile edit requires <id> <name> <url>"),
                    args.json,
                );
            }
            (
                "rule_profile.edit",
                json!({
                    "id": args.positional[2],
                    "name": args.positional[3],
                    "url": args.positional[4]
                }),
            )
        }
        "remove" => {
            if args.positional.len() != 3 {
                return print_error(
                    &BoxError::usage("rule-profile remove requires <id>"),
                    args.json,
                );
            }
            ("rule_profile.remove", json!({"id": args.positional[2]}))
        }
        "deactivate" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("rule-profile deactivate takes no arguments"),
                    args.json,
                );
            }
            ("rule_profile.deactivate", json!({}))
        }
        "fetch" => {
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage("rule-profile fetch [id] takes at most one id"),
                    args.json,
                );
            }
            let params = match args.positional.get(2) {
                Some(id) => json!({"id": id}),
                None => json!({}),
            };
            ("rule_profile.fetch", params)
        }
        "url" => {
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage("rule-profile url [id] takes at most one id"),
                    args.json,
                );
            }
            let params = match args.positional.get(2) {
                Some(id) => json!({"id": id}),
                None => json!({}),
            };
            ("rule_profile.url", params)
        }
        "set" => {
            if !(3..=4).contains(&args.positional.len()) {
                return print_error(
                    &BoxError::usage("rule-profile set requires <url> [name]"),
                    args.json,
                );
            }
            let mut params = Map::new();
            params.insert("url".to_string(), Value::String(args.positional[2].clone()));
            if let Some(name) = args.positional.get(3) {
                params.insert("name".to_string(), Value::String(name.clone()));
            }
            ("rule_profile.set", Value::Object(params))
        }
        "clear" => {
            if args.positional.len() != 2 {
                return print_error(
                    &BoxError::usage("rule-profile clear takes no arguments"),
                    args.json,
                );
            }
            ("rule_profile.clear", json!({}))
        }
        "update" => {
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage("rule-profile update [id] takes at most one id"),
                    args.json,
                );
            }
            let params = match args.positional.get(2) {
                Some(id) => json!({"id": id}),
                None => json!({}),
            };
            ("rule_profile.fetch", params)
        }
        "use" | "apply" => {
            if args.positional.len() > 3 {
                return print_error(
                    &BoxError::usage(format!("rule-profile {subcommand} [id] [--reload]")),
                    args.json,
                );
            }
            let mut params = Map::new();
            if let Some(id) = args.positional.get(2) {
                params.insert("id".to_string(), Value::String(id.clone()));
            }
            if args.reload {
                params.insert("reload".to_string(), Value::Bool(true));
            }
            (
                if subcommand == "use" {
                    "rule_profile.use"
                } else {
                    "rule_profile.apply"
                },
                Value::Object(params),
            )
        }
        other => {
            return print_error(
                &BoxError::usage(format!("unknown rule-profile command: {other}")),
                args.json,
            );
        }
    };
    match send_request(&socket, method, params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_internal(args: &Args) -> i32 {
    let Some(action) = args.positional.get(1).map(String::as_str) else {
        return print_error(&BoxError::usage("internal requires an action"), args.json);
    };
    if args.positional.len() != 2 {
        return print_error(
            &BoxError::usage("internal action takes no arguments"),
            args.json,
        );
    }
    let method = match action {
        "upgrade-prepare" => "internal.upgrade_prepare",
        "event-reset" => "internal.event_reset",
        "control-auto" => "internal.control_auto",
        "control-manual" => "internal.control_manual",
        _ => {
            return print_error(
                &BoxError::usage(format!("unknown internal action: {action}")),
                args.json,
            )
        }
    };
    let socket = socket_path(args);
    match send_request(&socket, method, json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_uninstall(args: &Args) -> i32 {
    let Some(action) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("uninstall requires preflight/apply/force"),
            args.json,
        );
    };
    if args.positional.len() != 2 || !matches!(action, "preflight" | "apply" | "force") {
        return print_error(
            &BoxError::usage("uninstall preflight|apply|force"),
            args.json,
        );
    }
    let socket = socket_path(args);
    let (method, params) = match action {
        "preflight" => ("uninstall.preflight", json!({})),
        "apply" => ("uninstall.apply", json!({})),
        _ => ("uninstall.force", json!({"confirm": true})),
    };
    match send_request(&socket, method, params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_purge(args: &Args) -> i32 {
    let Some(action) = args.positional.get(1).map(String::as_str) else {
        return print_error(&BoxError::usage("purge requires plan/apply"), args.json);
    };
    let socket = socket_path(args);
    let (method, params) = match action {
        "plan" if args.positional.len() == 2 => ("purge.plan", json!({})),
        "apply" if args.positional.len() == 3 => (
            "purge.apply",
            json!({"token": args.positional[2], "confirm": true}),
        ),
        _ => {
            return print_error(
                &BoxError::usage("purge plan | purge apply <token>"),
                args.json,
            )
        }
    };
    match send_request(&socket, method, params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_upgrade(args: &Args) -> i32 {
    let Some(action) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("upgrade requires status/confirm"),
            args.json,
        );
    };
    if args.positional.len() != 2 || !matches!(action, "status" | "confirm") {
        return print_error(&BoxError::usage("upgrade status|confirm"), args.json);
    }
    let socket = socket_path(args);
    let (method, params) = match action {
        "status" => ("upgrade.status", json!({})),
        _ => ("upgrade.confirm", json!({"confirm": true})),
    };
    match send_request(&socket, method, params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_service(args: &Args) -> i32 {
    let Some(action) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("service requires start/stop/restart/reload"),
            args.json,
        );
    };
    if !matches!(action, "start" | "stop" | "restart" | "reload") || args.positional.len() != 2 {
        return print_error(
            &BoxError::usage("service start|stop|restart|reload"),
            args.json,
        );
    }
    let socket = socket_path(args);
    let method = format!("service.{action}");
    match send_request(&socket, &method, json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_check(args: &Args) -> i32 {
    let socket = socket_path(args);
    match send_request(&socket, "config.check", json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) if error.code == ErrorCode::ManagerUnavailable => {
            let paths = Paths::resolve(args.root.clone());
            match crate::service::check_config(&paths) {
                Ok(data) => finish(Response::success(data, "local", 0, None), args.json),
                Err(err) => print_error(&err, args.json),
            }
        }
        Err(error) => print_error(&error, args.json),
    }
}

fn run_bond(args: &Args, profile: u8) -> i32 {
    let socket = socket_path(args);
    let method = format!("network.bond{profile}");
    match send_request(&socket, &method, json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) if error.code == ErrorCode::ManagerUnavailable => {
            match crate::network::apply_bond(profile) {
                Ok(data) => finish(Response::success(data, "local", 0, None), args.json),
                Err(err) => print_error(&err, args.json),
            }
        }
        Err(error) => print_error(&error, args.json),
    }
}

fn run_task(args: &Args) -> i32 {
    let Some(subcommand) = args.positional.get(1).map(String::as_str) else {
        return print_error(
            &BoxError::usage("task requires start/list/status/logs/cancel"),
            args.json,
        );
    };
    let socket = socket_path(args);
    let (method, params) = match subcommand {
        "start" => {
            if !(3..=4).contains(&args.positional.len()) {
                return print_error(
                    &BoxError::usage("task start requires <kind> [arg]"),
                    args.json,
                );
            }
            let mut params = Map::new();
            params.insert(
                "kind".to_string(),
                Value::String(args.positional[2].clone()),
            );
            if let Some(arg) = args.positional.get(3) {
                params.insert("arg".to_string(), Value::String(arg.clone()));
            }
            if args.wait {
                params.insert("wait".to_string(), Value::Bool(true));
            }
            ("task.start", Value::Object(params))
        }
        "list" => {
            if args.positional.len() != 2 {
                return print_error(&BoxError::usage("task list takes no arguments"), args.json);
            }
            ("task.list", json!({}))
        }
        "status" => {
            if args.positional.len() != 3 {
                return print_error(&BoxError::usage("task status requires <id>"), args.json);
            }
            ("task.status", json!({"id": args.positional[2]}))
        }
        "logs" => {
            if !(3..=4).contains(&args.positional.len()) {
                return print_error(
                    &BoxError::usage("task logs requires <id> [lines]"),
                    args.json,
                );
            }
            let mut params = Map::new();
            params.insert("id".to_string(), Value::String(args.positional[2].clone()));
            if let Some(lines) = args.positional.get(3) {
                let lines = match lines.parse::<u64>() {
                    Ok(value) => value,
                    Err(_) => {
                        return print_error(
                            &BoxError::usage("task logs lines must be a number"),
                            args.json,
                        );
                    }
                };
                params.insert("lines".to_string(), Value::Number(lines.into()));
            }
            ("task.logs", Value::Object(params))
        }
        "cancel" => {
            if args.positional.len() != 3 {
                return print_error(&BoxError::usage("task cancel requires <id>"), args.json);
            }
            ("task.cancel", json!({"id": args.positional[2]}))
        }
        other => {
            return print_error(
                &BoxError::usage(format!("unknown task command: {other}")),
                args.json,
            );
        }
    };
    match send_request(&socket, method, params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_task_runner(args: &Args) -> i32 {
    if args.positional.len() != 2 {
        return print_error(&BoxError::usage("task-run requires a task id"), args.json);
    }
    let paths = Paths::resolve(args.root.clone());
    task::run_task(&paths, &args.positional[1])
}

fn run_logs(args: &Args) -> i32 {
    let socket = socket_path(args);
    if args.positional.get(1).map(String::as_str) == Some("clear") {
        if args.positional.len() != 2 {
            return print_error(&BoxError::usage("logs clear takes no arguments"), args.json);
        }
        return match send_request(&socket, "logs.clear", json!({}), args.timeout) {
            Ok(response) => finish(response, args.json),
            Err(error) => print_error(&error, args.json),
        };
    }
    if args.positional.len() > 2 {
        return print_error(
            &BoxError::usage("logs [n] takes at most one number"),
            args.json,
        );
    }
    let lines = match args.positional.get(1) {
        Some(value) => match value.parse::<u64>() {
            Ok(value) => Some(value),
            Err(_) => {
                return print_error(&BoxError::usage("logs lines must be a number"), args.json);
            }
        },
        None => None,
    };
    let params = match lines {
        Some(value) => json!({"lines": value}),
        None => json!({}),
    };
    match send_request(&socket, "logs.get", params, args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_reset(args: &Args) -> i32 {
    if args.positional.len() != 2 || args.positional[1] != "subscriptions" {
        return print_error(
            &BoxError::usage("reset supports: reset subscriptions"),
            args.json,
        );
    }
    let socket = socket_path(args);
    match send_request(&socket, "reset.subscriptions", json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) => print_error(&error, args.json),
    }
}

fn run_status(args: &Args) -> i32 {
    if args.positional.len() != 1 {
        return print_error(&BoxError::usage("status takes no arguments"), args.json);
    }
    let socket = socket_path(args);
    match send_request(&socket, "status.get", json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) if error.code == ErrorCode::ManagerUnavailable => {
            let paths = Paths::resolve(args.root.clone());
            let data = json!({
                "daemon": false,
                "degraded": true,
                "running": false,
                "pid": Value::Null,
                "core": "mihomo",
                "mode": "unknown",
                "config": paths.config.display().to_string(),
                "config_exists": paths.config.exists(),
                "subscriptions": 0,
                "active_tasks": 0,
                "revision": 0,
                "upgrade": {
                    "required": false,
                    "confirmed": true,
                    "from": Value::Null,
                    "to": Value::Null
                },
                "capabilities": {},
                "warnings": [{"code": error.code.as_str(), "message": error.message}]
            });
            finish(Response::success(data, "local", 0, None), args.json)
        }
        Err(error) => print_error(&error, args.json),
    }
}

fn run_doctor(args: &Args) -> i32 {
    if args.positional.len() != 1 {
        return print_error(&BoxError::usage("doctor takes no arguments"), args.json);
    }
    let socket = socket_path(args);
    match send_request(&socket, "doctor.check", json!({}), args.timeout) {
        Ok(response) => finish(response, args.json),
        Err(error) if error.code == ErrorCode::ManagerUnavailable => {
            let paths = Paths::resolve(args.root.clone());
            match crate::doctor::check(&paths, false) {
                Ok(data) => finish(Response::success(data, "local", 0, None), args.json),
                Err(error) => print_error(&error, args.json),
            }
        }
        Err(error) => print_error(&error, args.json),
    }
}

fn run_version(args: &Args) -> i32 {
    if args.positional.len() != 1 {
        return print_error(&BoxError::usage("version takes no arguments"), args.json);
    }
    let data = json!({
        "server_version": env!("CARGO_PKG_VERSION"),
        "contract_version": crate::protocol::CONTRACT_VERSION,
        "protocol": PROTOCOL_VERSION,
        "commit": option_env!("BOXCTL_COMMIT").unwrap_or("unknown")
    });
    finish(Response::success(data, "local", 0, None), args.json)
}

fn read_json_input(args: &Args) -> Result<Value> {
    let file_arg = args.positional.get(2);
    if args.stdin && file_arg.is_some() {
        return Err(BoxError::usage(
            "apply accepts either a file or --stdin, not both",
        ));
    }
    if !args.stdin && file_arg.is_none() {
        return Err(BoxError::usage("apply requires a JSON file or --stdin"));
    }
    if args.positional.len() > 3 {
        return Err(BoxError::usage("apply takes at most one file"));
    }
    let source = if args.stdin {
        let mut source = String::new();
        io::stdin().read_to_string(&mut source)?;
        source
    } else {
        let path = file_arg.ok_or_else(|| BoxError::usage("missing JSON file"))?;
        fs::read_to_string(path)
            .map_err(|error| BoxError::from(error).with_detail(format!("cannot read {path}")))?
    };
    if source.trim().is_empty() {
        return Err(BoxError::usage("apply input is empty"));
    }
    let value: Value = serde_json::from_str(&source).map_err(|error| {
        BoxError::validation("apply input is not valid JSON").with_detail(error.to_string())
    })?;
    if !value.is_object() {
        return Err(BoxError::validation("apply input must be a JSON object"));
    }
    Ok(value)
}

fn socket_path(args: &Args) -> PathBuf {
    Paths::resolve(args.root.clone()).socket_for(args.socket.as_deref())
}

fn send_request(socket: &Path, method: &str, params: Value, timeout: Duration) -> Result<Response> {
    let mut stream = UnixStream::connect(socket).map_err(|error| {
        BoxError::manager_unavailable("boxctl daemon is unavailable")
            .with_detail(format!("{}: {error}", socket.display()))
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = Request {
        id: format!("cli-{}", util::now_ms()),
        method: method.to_string(),
        params: Some(params),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol: PROTOCOL_VERSION,
    };
    let encoded = serde_json::to_string(&request).map_err(|error| {
        BoxError::internal("cannot encode request").with_detail(error.to_string())
    })?;
    stream.write_all(encoded.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut limited = reader.by_ref().take((MAX_FRAME_BYTES + 1) as u64);
    let read = limited.read_line(&mut line)?;
    if read == 0 {
        return Err(BoxError::protocol(
            "daemon closed the connection without a response",
        ));
    }
    if line.len() > MAX_FRAME_BYTES {
        return Err(BoxError::protocol("response exceeds 1 MiB"));
    }
    serde_json::from_str(&line).map_err(|error| {
        BoxError::protocol("daemon response is not valid v2 JSON").with_detail(error.to_string())
    })
}

fn finish(response: Response, json_mode: bool) -> i32 {
    let exit = exit_for(&response);
    if json_mode {
        match response.encode() {
            Ok(line) => {
                let _ = util::write_stdout(line.as_bytes());
            }
            Err(error) => eprintln!("{}", error),
        }
        return exit;
    }
    if response.ok {
        if let Some(data) = &response.data {
            match serde_json::to_string_pretty(data) {
                Ok(text) => println!("{text}"),
                Err(error) => eprintln!("cannot render response: {error}"),
            }
        }
    } else if let Some(error) = &response.error {
        eprintln!("{}: {}", error.code, error.message);
        if let Some(detail) = &error.detail {
            eprintln!("{detail}");
        }
    }
    exit
}

fn print_error(error: &BoxError, json_mode: bool) -> i32 {
    if json_mode {
        let response = Response::error(error.clone(), "", 0);
        if let Ok(line) = response.encode() {
            let _ = util::write_stdout(line.as_bytes());
        }
    } else {
        eprintln!("{}", error);
    }
    error.code.exit_code()
}

fn exit_for(response: &Response) -> i32 {
    if response.ok {
        return 0;
    }
    response
        .error
        .as_ref()
        .and_then(|error| ErrorCode::from_code(&error.code))
        .map_or(1, ErrorCode::exit_code)
}

fn parse_args(raw: Vec<String>) -> Result<Args> {
    let mut args = Args {
        json: false,
        socket: None,
        root: None,
        timeout: Duration::from_secs(30),
        if_revision: None,
        stdin: false,
        reload: false,
        wait: false,
        positional: Vec::new(),
    };
    let mut index = 0;
    while index < raw.len() {
        let argument = &raw[index];
        match argument.as_str() {
            "--json" => args.json = true,
            "--stdin" => args.stdin = true,
            "--reload" => args.reload = true,
            "--wait" => args.wait = true,
            "--socket" | "--root" | "--timeout" | "--if-revision" => {
                index += 1;
                let Some(value) = raw.get(index) else {
                    return Err(BoxError::usage(format!("{argument} requires a value")));
                };
                match argument.as_str() {
                    "--socket" => args.socket = Some(PathBuf::from(value)),
                    "--root" => args.root = Some(PathBuf::from(value)),
                    "--timeout" => args.timeout = parse_timeout(value)?,
                    "--if-revision" => {
                        args.if_revision = Some(value.parse::<u64>().map_err(|_| {
                            BoxError::usage("--if-revision requires a non-negative integer")
                        })?);
                    }
                    _ => {}
                }
            }
            value if value.starts_with('-') => {
                return Err(BoxError::usage(format!("unknown option: {value}")));
            }
            value => args.positional.push(value.to_string()),
        }
        index += 1;
    }
    Ok(args)
}

fn parse_timeout(value: &str) -> Result<Duration> {
    let (number, multiplier) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1u64)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1000u64)
    } else {
        (value, 1000u64)
    };
    let amount = number
        .parse::<u64>()
        .map_err(|_| BoxError::usage("--timeout requires a number, optionally with ms or s"))?;
    if amount == 0 {
        return Err(BoxError::usage("--timeout must be greater than zero"));
    }
    Ok(Duration::from_millis(amount.saturating_mul(multiplier)))
}

#[cfg(test)]
mod tests {
    use super::{parse_args, parse_timeout};

    #[test]
    fn parses_global_options_anywhere() {
        let args = parse_args(vec![
            "settings".to_string(),
            "get".to_string(),
            "--json".to_string(),
            "--socket".to_string(),
            "/tmp/test.sock".to_string(),
        ])
        .expect("parse");
        assert!(args.json);
        assert_eq!(args.positional, vec!["settings", "get"]);
        assert_eq!(
            args.socket.expect("socket").to_string_lossy(),
            "/tmp/test.sock"
        );
        let reload = parse_args(vec![
            "rule-profile".to_string(),
            "use".to_string(),
            "id".to_string(),
            "--reload".to_string(),
        ])
        .expect("reload");
        assert!(reload.reload);
        assert_eq!(reload.positional, vec!["rule-profile", "use", "id"]);
    }

    #[test]
    fn parses_timeout_units() {
        assert_eq!(parse_timeout("2s").expect("timeout").as_secs(), 2);
        assert_eq!(parse_timeout("250ms").expect("timeout").as_millis(), 250);
        assert_eq!(parse_timeout("3").expect("timeout").as_secs(), 3);
        assert!(parse_timeout("0").is_err());
    }
}
