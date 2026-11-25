use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

struct Daemon {
    child: Child,
    root: PathBuf,
    socket: PathBuf,
}

impl Daemon {
    fn start() -> Self {
        let root = temp_root();
        init_root(&root);
        let socket = root.join("run").join("boxctl").join("control.sock");
        let child = Command::new(env!("CARGO_BIN_EXE_boxctl"))
            .arg("daemon")
            .arg("--root")
            .arg(&root)
            .arg("--socket")
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");
        let mut daemon = Self {
            child,
            root,
            socket,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if daemon.socket.exists() {
                return daemon;
            }
            if let Some(status) = daemon.child.try_wait().expect("wait daemon") {
                panic!("daemon exited before socket was ready: {status}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "daemon did not create socket at {}",
            daemon.socket.display()
        );
    }

    fn request(&self, method: &str, params: Value) -> Value {
        request_at(&self.socket, method, params).expect("request")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn temp_root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_nanos());
    let root = std::env::temp_dir().join(format!("boxctl-it-{}-{suffix}", std::process::id()));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn init_root(root: &Path) {
    let repo = repo_root();
    fs::copy(
        repo.join("box/settings.default.json"),
        root.join("settings.default.json"),
    )
    .expect("copy defaults");
    fs::copy(
        repo.join("box/settings.schema.json"),
        root.join("settings.schema.json"),
    )
    .expect("copy schema");
    fs::copy(
        repo.join("box/settings.default.json"),
        root.join("settings.json"),
    )
    .expect("copy settings");
    fs::set_permissions(
        root.join("settings.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("chmod settings");

    fs::create_dir_all(root.join("mihomo")).expect("create mihomo dir");
    fs::copy(
        repo.join("box/mihomo/config.yaml"),
        root.join("mihomo/config.yaml"),
    )
    .expect("copy config");
    fs::copy(
        repo.join("box/package.list.cfg"),
        root.join("package.list.cfg"),
    )
    .expect("copy package list");
    fs::copy(repo.join("box/gid.list.cfg"), root.join("gid.list.cfg")).expect("copy gid list");
    fs::copy(repo.join("box/ap.list.cfg"), root.join("ap.list.cfg")).expect("copy ap list");
}

fn request_at(socket: &Path, method: &str, params: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    let request = json!({
        "id": "it",
        "method": method,
        "params": params,
        "client_version": "test",
        "protocol": 1
    });
    let mut text = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    text.push('\n');
    stream
        .write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&line).map_err(|error| error.to_string())
}

fn raw_request(socket: &Path, text: &str) -> Value {
    let mut stream = UnixStream::connect(socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("timeout");
    stream.write_all(text.as_bytes()).expect("write");
    stream.write_all(b"\n").expect("newline");
    stream.flush().expect("flush");
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .expect("read response");
    serde_json::from_str(&line).expect("response json")
}

fn cli(socket: &Path, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_boxctl"))
        .args(args)
        .arg("--socket")
        .arg(socket)
        .output()
        .expect("run cli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (output.status.code().unwrap_or(1), value, stderr)
}

#[test]
fn settings_get_is_flat_v2_and_has_no_token_preview() {
    let daemon = Daemon::start();
    let response = daemon.request("settings.get", json!({}));
    assert_eq!(response["ok"], true);
    assert_eq!(response["meta"]["version"], 2);
    assert_eq!(response["meta"]["protocol"], 1);
    assert_eq!(response["data"]["network_mode"], "tun");
    assert_eq!(response["data"]["sensitive"]["githubtoken"]["set"], false);
    assert!(response["data"]["sensitive"]["githubtoken"]
        .get("preview")
        .is_none());

    let (code, cli_response, stderr) = cli(&daemon.socket, &["settings", "get", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(cli_response["ok"], true);
    assert_eq!(cli_response["data"]["network_mode"], "tun");
    assert!(cli_response["data"]["sensitive"]["githubtoken"]
        .get("preview")
        .is_none());
}

#[test]
fn settings_get_output_can_be_applied_with_sensitive_metadata() {
    let daemon = Daemon::start();
    let current = daemon.request("settings.get", json!({}));
    let file = daemon.root.join("apply-from-get.json");
    fs::write(
        &file,
        serde_json::to_vec(&json!({"settings": current["data"]})).expect("json"),
    )
    .expect("write apply file");
    let (code, value, stderr) = cli(
        &daemon.socket,
        &["--json", "settings", "apply", file.to_str().expect("path")],
    );
    assert_eq!(code, 0, "{value} {stderr}");
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["data"]["changed"], json!([]));
}

#[test]
fn settings_apply_set_token_and_revision_conflict() {
    let daemon = Daemon::start();
    let response = daemon.request(
        "settings.apply",
        json!({
            "settings": {"tproxy_port": "7777"},
            "githubtoken": "secret-token",
            "if_revision": 0
        }),
    );
    assert_eq!(response["ok"], true);
    assert_eq!(response["meta"]["revision"], 1);
    let changed = response["data"]["changed"].as_array().expect("changed");
    assert!(changed.iter().any(|value| value == "tproxy_port"));
    assert!(changed.iter().any(|value| value == "githubtoken"));

    let get = daemon.request("settings.get", json!({}));
    assert_eq!(get["data"]["tproxy_port"], "7777");
    assert_eq!(get["data"]["sensitive"]["githubtoken"]["set"], true);
    assert!(get["data"]["sensitive"]["githubtoken"]
        .get("preview")
        .is_none());
    assert_eq!(
        fs::read_to_string(daemon.root.join("githubtoken")).expect("token"),
        "secret-token"
    );
    assert_eq!(
        fs::metadata(daemon.root.join("githubtoken"))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let clear = daemon.request(
        "settings.apply",
        json!({"settings": {}, "githubtoken": "", "if_revision": 1}),
    );
    assert_eq!(clear["ok"], true);
    assert!(clear["data"]["changed"]
        .as_array()
        .expect("changed")
        .iter()
        .any(|value| value == "githubtoken"));
    assert!(!daemon.root.join("githubtoken").exists());
    let after_clear = daemon.request("settings.get", json!({}));
    assert_eq!(
        after_clear["data"]["sensitive"]["githubtoken"]["set"],
        false
    );

    let stale = daemon.request(
        "settings.apply",
        json!({"settings": {"tproxy_port": "8888"}, "if_revision": 0}),
    );
    assert_eq!(stale["ok"], false);
    assert_eq!(stale["error"]["code"], "E_CONFLICT");

    let before = fs::read(daemon.root.join("settings.json")).expect("read before");
    let unknown = daemon.request(
        "settings.apply",
        json!({"settings": {"not_a_setting": "x"}}),
    );
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["error"]["code"], "E_VALIDATION");
    let after = fs::read(daemon.root.join("settings.json")).expect("read after");
    assert_eq!(before, after);
}

#[test]
fn settings_set_and_cli_file_apply() {
    let daemon = Daemon::start();
    let set = daemon.request(
        "settings.set",
        json!({"key": "network_mode", "value": "tproxy"}),
    );
    assert_eq!(set["ok"], true);
    let get = daemon.request("settings.get", json!({}));
    assert_eq!(get["data"]["network_mode"], "tproxy");

    let file = daemon.root.join("apply.json");
    fs::write(&file, r#"{"settings":{"redir_port":"9999"}}"#).expect("write file");
    let (code, response, stderr) = cli(
        &daemon.socket,
        &["settings", "apply", file.to_str().expect("path"), "--json"],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(response["ok"], true);
    let get = daemon.request("settings.get", json!({}));
    assert_eq!(get["data"]["redir_port"], "9999");
}

#[test]
fn protocol_errors_are_v2_envelopes() {
    let daemon = Daemon::start();
    let malformed = raw_request(&daemon.socket, "{not-json");
    assert_eq!(malformed["ok"], false);
    assert_eq!(malformed["error"]["code"], "E_PROTOCOL");
    assert_eq!(malformed["meta"]["version"], 2);

    let bad_protocol = raw_request(
        &daemon.socket,
        r#"{"id":"x","method":"settings.get","params":{},"client_version":"t","protocol":2}"#,
    );
    assert_eq!(bad_protocol["ok"], false);
    assert_eq!(bad_protocol["error"]["code"], "E_PROTOCOL");

    let unknown = daemon.request("no.such.method", json!({}));
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["error"]["code"], "E_NOT_FOUND");
}

#[test]
fn socket_and_run_directory_are_private() {
    let daemon = Daemon::start();
    let socket_mode = fs::metadata(&daemon.socket)
        .expect("socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(socket_mode, 0o600);
    let run_mode = fs::metadata(daemon.socket.parent().expect("parent"))
        .expect("run metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(run_mode, 0o700);
    let lock_mode = fs::metadata(daemon.root.join("run/boxctl/daemon.lock"))
        .expect("lock metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(lock_mode, 0o600);
    let daemon_pid_mode = fs::metadata(daemon.root.join("run/boxctl/daemon.pid"))
        .expect("daemon pid metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(daemon_pid_mode, 0o600);
}

#[test]
fn version_is_local_and_status_works_online() {
    let (code, version, stderr) = cli(Path::new("/nonexistent.sock"), &["version", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(version["ok"], true);
    assert_eq!(version["data"]["contract_version"], 2);
    assert_eq!(version["data"]["protocol"], 1);

    let daemon = Daemon::start();
    let (code, status, stderr) = cli(&daemon.socket, &["status", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(status["ok"], true);
    assert_eq!(status["data"]["daemon"], true);
    assert_eq!(status["data"]["running"], false);
}

#[test]
fn config_core_lists_round_trip_and_cli() {
    let daemon = Daemon::start();
    let config = daemon.request("config.get", json!({}));
    assert_eq!(config["ok"], true);
    assert!(config["data"]["proxy-groups"].is_array());
    assert!(config["data"]["rule-providers"].is_object());
    assert!(config["data"]["rules"].is_array());

    let applied = daemon.request(
        "config.apply",
        json!({
            "proxy-groups": [{"name": "TEST", "type": "select", "proxies": ["DIRECT"]}],
            "rule-providers": {
                "custom": {
                    "type": "http",
                    "behavior": "domain",
                    "url": "https://example.com/custom.yaml",
                    "path": "./rules/custom.yaml"
                }
            },
            "rules": ["RULE-SET,custom,TEST", "MATCH,DIRECT"]
        }),
    );
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["meta"]["revision"], 1);
    let after = daemon.request("config.get", json!({}));
    assert_eq!(after["data"]["proxy-groups"][0]["name"], "TEST");

    let before = fs::read(daemon.root.join("mihomo/config.yaml")).expect("before");
    let invalid = daemon.request(
        "config.apply",
        json!({
            "proxy-groups": [{"name": "TEST", "type": "select", "proxies": ["DIRECT"]}],
            "rule-providers": {},
            "rules": ["RULE-SET,missing,TEST"]
        }),
    );
    assert_eq!(invalid["ok"], false);
    assert_eq!(invalid["error"]["code"], "E_VALIDATION");
    assert_eq!(
        fs::read(daemon.root.join("mihomo/config.yaml")).expect("after"),
        before
    );

    let core = daemon.request("core.get", json!({}));
    assert_eq!(core["ok"], true);
    assert_eq!(core["data"]["structured"]["mode"], "rule");
    assert!(core["data"]["raw"]
        .as_str()
        .expect("raw")
        .contains("mixed-port"));
    let core_applied = daemon.request(
        "core.apply",
        json!({"structured": {"mode": "global", "log-level": "info", "allow-lan": false}}),
    );
    assert_eq!(core_applied["ok"], true, "{core_applied}");
    assert_eq!(core_applied["data"]["mode"], "structured");
    let core_after = daemon.request("core.get", json!({}));
    assert_eq!(core_after["data"]["structured"]["mode"], "global");
    assert_eq!(core_after["data"]["structured"]["log-level"], "info");

    let lists = daemon.request(
        "lists.apply",
        json!({
            "package": [{"uid": "0", "package": "com.example.app"}],
            "gid": ["10450"],
            "interfaces": {"allow": ["wlan+"], "ignore": []}
        }),
    );
    assert_eq!(lists["ok"], true, "{lists}");
    let lists_after = daemon.request("lists.get", json!({}));
    assert_eq!(lists_after["data"]["package"][0]["uid"], "0");
    assert_eq!(lists_after["data"]["gid"][0], "10450");
    assert_eq!(lists_after["data"]["interfaces"]["allow"][0], "wlan+");

    let before_package = fs::read(daemon.root.join("package.list.cfg")).expect("package before");
    let bad_lists = daemon.request(
        "lists.apply",
        json!({"package": [{"package": "bad package"}], "gid": ["999"]}),
    );
    assert_eq!(bad_lists["ok"], false);
    assert_eq!(bad_lists["error"]["code"], "E_VALIDATION");
    assert_eq!(
        fs::read(daemon.root.join("package.list.cfg")).expect("package after"),
        before_package
    );

    let (code, cli_config, stderr) = cli(&daemon.socket, &["config", "get", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(cli_config["data"]["proxy-groups"][0]["name"], "TEST");

    let deleted = daemon.request("config.delete", json!({}));
    assert_eq!(deleted["ok"], true, "{deleted}");
    assert!(!daemon.root.join("mihomo/config.yaml").exists());
    assert!(!deleted["data"]["path"]
        .as_str()
        .expect("deleted path")
        .is_empty());
}

#[test]
fn subscription_management_round_trip() {
    let daemon = Daemon::start();
    let empty = daemon.request("sub.list", json!({}));
    assert_eq!(empty["ok"], true);
    assert!(empty["data"]["items"].as_array().expect("items").is_empty());

    let added = daemon.request(
        "sub.add",
        json!({
            "url": "https://example.com/sub?token=secret",
            "name": "main",
            "user_agent": "agent/1.0"
        }),
    );
    assert_eq!(added["ok"], true, "{added}");
    assert_eq!(added["data"]["name"], "main");
    assert_eq!(added["data"]["url_masked"], "https://example.com/***");
    assert_eq!(added["data"]["user_agent_effective"], "agent/1.0");

    let duplicate = daemon.request(
        "sub.add",
        json!({"url": "https://example.com/other", "name": "main"}),
    );
    assert_eq!(duplicate["ok"], false);
    assert_eq!(duplicate["error"]["code"], "E_CONFLICT");

    let disabled = daemon.request("sub.disable", json!({"name": "main"}));
    assert_eq!(disabled["ok"], true, "{disabled}");
    assert_eq!(disabled["data"]["enabled"], false);
    let detail = daemon.request("sub.get", json!({"name": "main"}));
    assert_eq!(detail["data"]["enabled"], false);

    let enabled = daemon.request("sub.enable", json!({"name": "main"}));
    assert_eq!(enabled["ok"], true);
    assert_eq!(enabled["data"]["enabled"], true);

    let changed = daemon.request(
        "sub.set",
        json!({
            "old_name": "main",
            "new_name": "renamed",
            "url": "https://example.com/new",
            "user_agent": ""
        }),
    );
    assert_eq!(changed["ok"], true, "{changed}");
    assert_eq!(changed["data"]["name"], "renamed");
    assert_eq!(changed["data"]["renamed"], true);
    assert_eq!(
        changed["data"]["user_agent_effective"],
        "clash-verge/v2.4.5"
    );

    let (code, cli_list, stderr) = cli(&daemon.socket, &["sub", "list", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(cli_list["data"]["items"][0]["name"], "renamed");

    let removed = daemon.request("sub.rm", json!({"name": "renamed"}));
    assert_eq!(removed["ok"], true);
    let empty = daemon.request("sub.list", json!({}));
    assert!(empty["data"]["items"].as_array().expect("items").is_empty());
}

#[test]
fn logs_and_subscription_reset() {
    let daemon = Daemon::start();
    fs::write(daemon.root.join("run/runs.log"), "one\ntwo\nthree\n").expect("write log");
    let logs = daemon.request("logs.get", json!({"lines": 2}));
    assert_eq!(logs["ok"], true);
    assert_eq!(logs["data"]["lines"], json!(["two", "three"]));
    let cleared = daemon.request("logs.clear", json!({}));
    assert_eq!(cleared["ok"], true);
    assert_eq!(
        cleared["data"]["cleared"]
            .as_array()
            .expect("cleared")
            .len(),
        1
    );

    let added = daemon.request(
        "sub.add",
        json!({"url": "https://example.com/sub", "name": "main"}),
    );
    assert_eq!(added["ok"], true);
    let reset = daemon.request("reset.subscriptions", json!({}));
    assert_eq!(reset["ok"], true);
    assert_eq!(reset["data"]["cleared"], true);
    let empty = daemon.request("sub.list", json!({}));
    assert!(empty["data"]["items"].as_array().expect("items").is_empty());
}

#[test]
fn rule_profile_local_round_trip() {
    let daemon = Daemon::start();
    let added = daemon.request(
        "rule_profile.add",
        json!({"name": "Local rules", "url": "https://example.com/rules.yaml"}),
    );
    assert_eq!(added["ok"], true, "{added}");
    let id = added["data"]["id"].as_str().expect("id").to_string();
    assert_eq!(added["data"]["url_masked"], "https://example.com/***");

    let listed = daemon.request("rule_profile.list", json!({}));
    assert_eq!(listed["data"]["items"][0]["id"], id);
    let detail = daemon.request("rule_profile.get", json!({}));
    assert_eq!(detail["data"]["configured"], true);

    let dir = daemon.root.join("mihomo/rule-profiles");
    fs::create_dir_all(&dir).expect("rule dir");
    fs::write(
        dir.join(format!("{id}.pending.yaml")),
        r#"proxy-groups:
  - name: TEST
    type: select
    proxies:
      - DIRECT
rule-providers:
  custom:
    type: http
    behavior: domain
    url: https://example.com/custom.yaml
    path: ./rules/custom.yaml
rules:
  - RULE-SET,custom,TEST
"#,
    )
    .expect("pending");
    let used = daemon.request("rule_profile.use", json!({"id": id}));
    assert_eq!(used["ok"], true, "{used}");
    assert_eq!(used["data"]["counts"]["groups"], 1);
    let config = daemon.request("config.get", json!({}));
    assert_eq!(config["data"]["proxy-groups"][0]["name"], "TEST");

    let (code, cli_list, stderr) = cli(&daemon.socket, &["rule-profile", "list", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(cli_list["data"]["items"][0]["active"], true);

    let deactivated = daemon.request("rule_profile.deactivate", json!({}));
    assert_eq!(deactivated["data"]["active"], false);
    let removed = daemon.request("rule_profile.remove", json!({"id": id}));
    assert_eq!(removed["ok"], true, "{removed}");
    let listed = daemon.request("rule_profile.list", json!({}));
    assert!(listed["data"]["items"]
        .as_array()
        .expect("items")
        .is_empty());
}

#[test]
fn task_status_logs_and_list() {
    let daemon = Daemon::start();
    let dir = daemon.root.join("run/tasks/20260101120000-1");
    fs::create_dir_all(&dir).expect("task dir");
    fs::write(dir.join("kind"), "sub-update\n").expect("kind");
    fs::write(dir.join("title"), "update\n").expect("title");
    fs::write(dir.join("state"), "running\n").expect("state");
    fs::write(dir.join("cancelable"), "true\n").expect("cancelable");
    fs::write(dir.join("pid"), "999999\n").expect("pid");
    fs::write(dir.join("started_at"), "2026-01-01 12:00:00\n").expect("started");
    fs::write(dir.join("output.log"), "one\ntwo\n").expect("output");

    let status = daemon.request("task.status", json!({"id": "20260101120000-1"}));
    assert_eq!(status["ok"], true, "{status}");
    assert_eq!(status["data"]["state"], "interrupted");
    let logs = daemon.request("task.logs", json!({"id": "20260101120000-1", "lines": 1}));
    assert_eq!(logs["data"]["lines"], json!(["two"]));
    let listed = daemon.request("task.list", json!({}));
    assert_eq!(listed["data"]["items"][0]["id"], "20260101120000-1");
    let (code, cli_task, stderr) = cli(&daemon.socket, &["task", "list", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(cli_task["data"]["items"][0]["id"], "20260101120000-1");
}

#[test]
fn task_start_runs_sub_update_runner() {
    let daemon = Daemon::start();
    let started = daemon.request("task.start", json!({"kind": "sub-update"}));
    assert_eq!(started["ok"], true, "{started}");
    let id = started["data"]["id"].as_str().expect("id").to_string();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut state = String::new();
    while Instant::now() < deadline {
        let status = daemon.request("task.status", json!({"id": id}));
        state = status["data"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if state == "success" || state == "failed" || state == "interrupted" {
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }
    assert_eq!(state, "success");
    let logs = daemon.request("task.logs", json!({"id": id, "lines": 20}));
    assert!(logs["data"]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .any(|line| { line.as_str().is_some_and(|text| text.contains("updated")) }));
}

#[test]
fn service_errors_are_structured() {
    let daemon = Daemon::start();
    let reload = daemon.request("service.reload", json!({}));
    assert_eq!(reload["ok"], false, "{reload}");
    assert_eq!(reload["error"]["code"], "E_RELOAD");
    let start = daemon.request("service.start", json!({}));
    assert_eq!(start["ok"], false, "{start}");
    assert_eq!(start["error"]["code"], "E_NOT_FOUND");
}

#[test]
fn upgrade_confirmation_is_persisted() {
    let daemon = Daemon::start();
    let idle = daemon.request("upgrade.status", json!({}));
    assert_eq!(idle["ok"], true, "{idle}");
    assert_eq!(idle["data"]["required"], false);
    fs::create_dir_all(daemon.root.join("state")).expect("state");
    fs::write(
        daemon.root.join("state/upgrade.json"),
        r#"{"required":true,"confirmed":false,"from":"1.4.0","to":"2.0.0"}"#,
    )
    .expect("upgrade state");
    let pending = daemon.request("upgrade.status", json!({}));
    assert_eq!(pending["data"]["required"], true);
    let confirmed = daemon.request("upgrade.confirm", json!({"confirm": true}));
    assert_eq!(confirmed["ok"], true, "{confirmed}");
    assert_eq!(confirmed["data"]["required"], false);
    assert_eq!(confirmed["data"]["confirmed"], true);
    let stored = fs::read_to_string(daemon.root.join("state/upgrade.json")).expect("stored");
    assert!(stored.contains("\"confirmed\": true"));
}

#[test]
fn uninstall_preflight_and_purge_plan_are_non_destructive() {
    let daemon = Daemon::start();
    let preflight = daemon.request("uninstall.preflight", json!({}));
    assert_eq!(preflight["ok"], true, "{preflight}");
    assert!(preflight["data"]["blockers"]
        .as_array()
        .expect("blockers")
        .is_empty());
    let planned = daemon.request("purge.plan", json!({}));
    assert_eq!(planned["ok"], true, "{planned}");
    let token = planned["data"]["token"].as_str().expect("token");
    let denied = daemon.request("purge.apply", json!({"token": token, "confirm": false}));
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "E_NEEDS_CONFIRMATION");
    assert!(daemon.root.join("settings.json").exists());
}

#[test]
fn purge_apply_deletes_confirmed_targets_and_preserves_other_files() {
    let daemon = Daemon::start();
    fs::write(daemon.root.join("githubtoken"), "secret").expect("token");
    fs::write(daemon.root.join("mihomo/subscriptions.list"), "proxy: []").expect("subs");
    fs::write(daemon.root.join("outside.txt"), "keep").expect("outside");
    let planned = daemon.request("purge.plan", json!({}));
    assert_eq!(planned["ok"], true, "{planned}");
    let token = planned["data"]["token"]
        .as_str()
        .expect("token")
        .to_string();
    let applied = daemon.request("purge.apply", json!({"token": token, "confirm": true}));
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["data"]["report"]["complete"], true);
    assert!(!daemon.root.join("settings.json").exists());
    assert!(!daemon.root.join("settings.default.json").exists());
    assert!(!daemon.root.join("mihomo/config.yaml").exists());
    assert!(!daemon.root.join("githubtoken").exists());
    assert!(!daemon.root.join("mihomo/subscriptions.list").exists());
    assert!(daemon.root.join("outside.txt").exists());
    assert!(daemon.socket.exists());
    let reused = daemon.request("purge.apply", json!({"token": token, "confirm": true}));
    assert_eq!(reused["ok"], false, "{reused}");
}

#[test]
fn internal_upgrade_prepare_backs_up_and_resets_settings() {
    let daemon = Daemon::start();
    let mut settings: Value = serde_json::from_str(
        &fs::read_to_string(daemon.root.join("settings.json")).expect("settings"),
    )
    .expect("json");
    settings["boot_auto_start"] = json!("true");
    fs::write(
        daemon.root.join("settings.json"),
        serde_json::to_vec_pretty(&settings).expect("serialize"),
    )
    .expect("write settings");
    fs::write(daemon.root.join("githubtoken"), "secret").expect("token");
    let result = daemon.request("internal.upgrade_prepare", json!({}));
    assert_eq!(result["ok"], true, "{result}");
    assert!(result["data"]["backup"].as_str().is_some());
    assert_eq!(result["data"]["token_cleared"], true);
    let reset: Value = serde_json::from_str(
        &fs::read_to_string(daemon.root.join("settings.json")).expect("settings"),
    )
    .expect("json");
    assert_eq!(reset["boot_auto_start"], "false");
    assert!(!daemon.root.join("githubtoken").exists());
}

#[test]
fn internal_event_reset_clears_manual_retry_state() {
    let daemon = Daemon::start();
    fs::create_dir_all(daemon.root.join("state")).expect("state");
    fs::write(
        daemon.root.join("state/event.json"),
        r#"{"failures":5,"max":5,"next_at":0,"last_error":"boom","manual":true}"#,
    )
    .expect("event state");
    let result = daemon.request("internal.event_reset", json!({}));
    assert_eq!(result["ok"], true, "{result}");
    assert!(!daemon.root.join("state/event.json").exists());
    let status = daemon.request("status.get", json!({}));
    assert_eq!(status["data"]["event_control"]["manual"], false);
}

#[test]
fn internal_control_auto_switches_mode_and_reports_status() {
    let daemon = Daemon::start();
    let result = daemon.request("internal.control_auto", json!({}));
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["mode"], "auto");
    let status = daemon.request("status.get", json!({}));
    assert_eq!(status["data"]["control"]["mode"], "auto");
}

#[test]
fn doctor_check_reports_structured_checks() {
    let daemon = Daemon::start();
    let result = daemon.request("doctor.check", json!({}));
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["daemon"], true);
    let checks = result["data"]["checks"].as_array().expect("checks");
    assert!(checks.iter().any(|item| item["name"] == "settings"));
}

#[test]
fn doctor_cli_works_without_daemon() {
    let root = temp_root();
    init_root(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_boxctl"))
        .args(["doctor", "--root"])
        .arg(&root)
        .arg("--json")
        .output()
        .expect("doctor");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["daemon"], false);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn task_start_wait_returns_terminal_state() {
    let daemon = Daemon::start();
    let result = daemon.request("task.start", json!({"kind": "sub-update", "wait": true}));
    assert_eq!(result["ok"], true, "{result}");
    assert_ne!(result["data"]["state"], "running", "{result}");
    assert!(["success", "failed", "interrupted"]
        .contains(&result["data"]["state"].as_str().unwrap_or("")));
}

#[test]
fn uninstall_apply_prepares_but_never_deletes_persistent_data() {
    let daemon = Daemon::start();
    fs::write(daemon.root.join("githubtoken"), "secret").expect("token");
    let result = daemon.request("uninstall.apply", json!({}));
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["status"], "manual_removal_required");
    assert_eq!(result["data"]["data_deleted"], false);
    assert!(daemon.root.join("settings.json").exists());
    assert!(daemon.root.join("githubtoken").exists());
}

#[test]
fn daemon_removes_socket_on_sigterm() {
    let mut daemon = Daemon::start();
    assert!(daemon.socket.exists());
    unsafe {
        libc::kill(daemon.child.id() as libc::pid_t, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = daemon.child.try_wait().expect("wait") {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(Instant::now() < deadline, "daemon did not exit on SIGTERM");
        thread::sleep(Duration::from_millis(50));
    }
    assert!(!daemon.socket.exists(), "socket was not removed");
    assert!(!daemon.root.join("run/boxctl/daemon.pid").exists());
}

#[test]
fn oversized_frame_returns_protocol_error() {
    let daemon = Daemon::start();
    let mut stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let payload = format!("{}\n", "a".repeat(1_100_000));
    let _ = stream.write_all(payload.as_bytes());
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read");
    let envelope: Value = serde_json::from_str(&line).expect("json");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "E_PROTOCOL");
}

#[test]
fn second_daemon_does_not_steal_active_socket() {
    let daemon = Daemon::start();
    let output = Command::new(env!("CARGO_BIN_EXE_boxctl"))
        .arg("daemon")
        .arg("--root")
        .arg(&daemon.root)
        .arg("--socket")
        .arg(&daemon.socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("second daemon");
    assert!(!output.status.success());
    let status = daemon.request("status.get", json!({}));
    assert_eq!(status["ok"], true, "{status}");
}
