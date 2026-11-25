use std::path::{Path, PathBuf};

use crate::error::{BoxError, Result};

pub const DEFAULT_HOME: &str = "/data/adb/box";

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub settings: PathBuf,
    pub defaults: PathBuf,
    pub schema: PathBuf,
    pub token: PathBuf,
    pub revision: PathBuf,
    pub run_dir: PathBuf,
    pub socket: PathBuf,
    pub pid_file: PathBuf,
    pub daemon_lock: PathBuf,
    pub daemon_pid: PathBuf,
    pub config: PathBuf,
    pub package_list: PathBuf,
    pub gid_list: PathBuf,
    pub ap_list: PathBuf,
    pub subscriptions: PathBuf,
    pub subscriptions_ua_dir: PathBuf,
    pub providers_dir: PathBuf,
    pub subscriptions_providers: PathBuf,
    pub runs_log: PathBuf,
    pub tool_log: PathBuf,
    pub core_log: PathBuf,
    pub rule_profiles_dir: PathBuf,
    pub rule_profile_active: PathBuf,
    pub tasks_dir: PathBuf,
    pub core_bin: PathBuf,
    pub geoip_file: PathBuf,
    pub geosite_file: PathBuf,
    pub box_pid: PathBuf,
    pub state_dir: PathBuf,
    pub tx_dir: PathBuf,
    pub candidates_dir: PathBuf,
    pub service_state: PathBuf,
    pub service_intent: PathBuf,
    pub last_good: PathBuf,
    pub schedule_state: PathBuf,
    pub upgrade_state: PathBuf,
    pub purge_state: PathBuf,
    pub event_stop: PathBuf,
    pub backup_dir: PathBuf,
    pub event_state: PathBuf,
    pub control_state: PathBuf,
    pub cnip_v4_file: PathBuf,
    pub cnip_v6_file: PathBuf,
}

impl Paths {
    pub fn resolve(root: Option<PathBuf>) -> Self {
        let home = root
            .or_else(|| {
                std::env::var_os("BOX_HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| PathBuf::from(DEFAULT_HOME));
        Self::new(home)
    }

    pub fn new(home: PathBuf) -> Self {
        let run_dir = home.join("run").join("boxctl");
        Self {
            settings: home.join("settings.json"),
            defaults: home.join("settings.default.json"),
            schema: home.join("settings.schema.json"),
            token: home.join("githubtoken"),
            revision: home.join("state").join("revision"),
            socket: run_dir.join("control.sock"),
            pid_file: run_dir.join("boxctl.pid"),
            daemon_lock: run_dir.join("daemon.lock"),
            daemon_pid: run_dir.join("daemon.pid"),
            config: home.join("mihomo").join("config.yaml"),
            package_list: home.join("package.list.cfg"),
            gid_list: home.join("gid.list.cfg"),
            ap_list: home.join("ap.list.cfg"),
            subscriptions: home.join("mihomo").join("subscriptions.list"),
            subscriptions_ua_dir: home.join("mihomo").join("subscriptions.ua"),
            providers_dir: home.join("mihomo").join("proxy_provider"),
            subscriptions_providers: home.join("run").join("subscriptions.providers"),
            runs_log: home.join("run").join("runs.log"),
            tool_log: home.join("run").join("tool.log"),
            core_log: home.join("run").join("mihomo.log"),
            rule_profiles_dir: home.join("mihomo").join("rule-profiles"),
            rule_profile_active: home.join("mihomo").join("rule-profiles").join("active"),
            tasks_dir: home.join("run").join("tasks"),
            core_bin: home.join("bin").join("mihomo"),
            geoip_file: home.join("mihomo").join("Country.mmdb"),
            geosite_file: home.join("mihomo").join("GeoSite.dat"),
            box_pid: home.join("run").join("box.pid"),
            state_dir: home.join("state"),
            tx_dir: home.join("state").join("tx"),
            candidates_dir: home.join("run").join("candidates"),
            service_state: home.join("state").join("service.json"),
            service_intent: home.join("state").join("service.intent"),
            last_good: home.join("state").join("last_good.json"),
            schedule_state: home.join("state").join("schedule.json"),
            upgrade_state: home.join("state").join("upgrade.json"),
            purge_state: home.join("state").join("purge.json"),
            event_stop: home.join("state").join("event_stop"),
            backup_dir: home.join("backup"),
            event_state: home.join("state").join("event.json"),
            control_state: home.join("state").join("control.json"),
            cnip_v4_file: home.join("run").join("state").join("cn_ip.txt"),
            cnip_v6_file: home.join("run").join("state").join("cn_ipv6.txt"),
            run_dir,
            home,
        }
    }

    pub fn socket_for(&self, socket: Option<&Path>) -> PathBuf {
        socket.map_or_else(|| self.socket.clone(), PathBuf::from)
    }

    pub fn ensure_home(&self) -> Result<()> {
        if self.home.as_os_str().is_empty() {
            return Err(BoxError::usage("Box home is empty"));
        }
        std::fs::create_dir_all(&self.home).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot create {}", self.home.display()))
        })
    }

    pub fn ensure_run_dir(&self) -> Result<()> {
        crate::util::ensure_dir(&self.run_dir, true)
    }

    pub fn ensure_state_dir(&self) -> Result<()> {
        crate::util::ensure_dir(
            self.revision
                .parent()
                .ok_or_else(|| BoxError::internal("revision path has no parent"))?,
            true,
        )
    }
}
