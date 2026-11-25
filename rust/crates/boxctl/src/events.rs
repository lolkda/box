use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Wifi,
    Cellular,
    Ethernet,
    None,
    Unknown,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wifi => "wifi",
            Self::Cellular => "cellular",
            Self::Ethernet => "ethernet",
            Self::None => "none",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkState {
    pub transport: Transport,
    pub iface: Option<String>,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub mac: Option<String>,
    pub validated: Option<bool>,
}

impl NetworkState {
    pub fn json(&self) -> Value {
        json!({
            "transport": self.transport.as_str(),
            "iface": self.iface,
            "ssid": self.ssid,
            "bssid": self.bssid,
            "mac": self.mac,
            "validated": self.validated
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EventTracker {
    stability: Duration,
    candidate: Option<NetworkState>,
    candidate_since: u64,
    emitted: Option<NetworkState>,
}

#[allow(dead_code)]
impl EventTracker {
    pub fn new(stability: Duration) -> Self {
        Self {
            stability,
            candidate: None,
            candidate_since: 0,
            emitted: None,
        }
    }

    pub fn observe(&mut self, state: NetworkState, now_millis: u64) -> Option<NetworkState> {
        if self.emitted.as_ref() == Some(&state) {
            self.candidate = None;
            return None;
        }
        if self.candidate.as_ref() != Some(&state) {
            self.candidate = Some(state);
            self.candidate_since = now_millis;
            return None;
        }
        if now_millis.saturating_sub(self.candidate_since) < self.stability.as_millis() as u64 {
            return None;
        }
        self.emitted = self.candidate.clone();
        self.candidate = None;
        self.emitted.clone()
    }

    pub fn last(&self) -> Option<&NetworkState> {
        self.emitted.as_ref()
    }
}

pub fn parse_route(output: &str) -> (Transport, Option<String>) {
    for line in output.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"default") {
            continue;
        }
        let iface = words
            .iter()
            .position(|word| *word == "dev")
            .and_then(|index| words.get(index + 1))
            .copied()
            .map(str::to_string);
        let transport = match iface.as_deref() {
            Some(value) if value.starts_with("wlan") => Transport::Wifi,
            Some(value) if value.starts_with("eth") => Transport::Ethernet,
            Some(value)
                if value.starts_with("rmnet")
                    || value.starts_with("ccmni")
                    || value.starts_with("pdp") =>
            {
                Transport::Cellular
            }
            Some(_) => Transport::Unknown,
            None => Transport::Unknown,
        };
        return (transport, iface);
    }
    (Transport::None, None)
}

fn transport_for_iface(iface: &str) -> Transport {
    if iface.starts_with("wlan") {
        Transport::Wifi
    } else if iface.starts_with("eth") {
        Transport::Ethernet
    } else if iface.starts_with("rmnet") || iface.starts_with("ccmni") || iface.starts_with("pdp") {
        Transport::Cellular
    } else {
        Transport::Unknown
    }
}

pub fn parse_route_get(output: &str) -> Option<(Transport, Option<String>)> {
    let words: Vec<&str> = output.split_whitespace().collect();
    let index = words.iter().position(|word| *word == "dev")?;
    let iface = words.get(index + 1)?.to_string();
    Some((transport_for_iface(&iface), Some(iface)))
}

fn parse_route_table(output: &str) -> Option<(Transport, Option<String>)> {
    let mut fallback = None;
    for line in output.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"default") {
            continue;
        }
        let Some(index) = words.iter().position(|word| *word == "dev") else {
            continue;
        };
        let Some(iface) = words.get(index + 1).copied() else {
            continue;
        };
        if iface == "meta" || iface == "dummy0" || iface == "lo" {
            continue;
        }
        let transport = transport_for_iface(iface);
        let known = transport != Transport::Unknown;
        let candidate = (transport, Some(iface.to_string()));
        if known {
            return Some(candidate);
        }
        fallback.get_or_insert(candidate);
    }
    fallback
}

pub fn parse_wifi_iface(output: &str) -> Option<String> {
    let marker = "iface=";
    let start = output.find(marker)? + marker.len();
    let rest = &output[start..];
    let iface: String = rest
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    (!iface.is_empty()).then_some(iface)
}

pub fn parse_wifi_status(output: &str) -> (Option<String>, Option<String>, bool) {
    let ssid = field(output, "SSID:");
    let bssid = field(output, "BSSID:");
    let unknown = ssid.is_none() || ssid.as_deref() == Some("<unknown ssid>");
    (
        ssid.filter(|value| value != "<unknown ssid>" && !value.is_empty()),
        bssid.filter(|value| value != "02:00:00:00:00:00"),
        unknown,
    )
}

fn field(output: &str, key: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(index) = line.find(key) {
            let rest = line[index + key.len()..]
                .trim_start()
                .trim_start_matches('=')
                .trim_start();
            let value = if rest.starts_with("<unknown") {
                "<unknown ssid>".to_string()
            } else if let Some(rest) = rest.strip_prefix('"') {
                rest.split('"').next().unwrap_or_default().to_string()
            } else if let Some(rest) = rest.strip_prefix('\'') {
                rest.split('\'').next().unwrap_or_default().to_string()
            } else {
                rest.split(|character: char| character == ',' || character.is_whitespace())
                    .next()
                    .unwrap_or_default()
                    .to_string()
            };
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

pub fn parse_mac(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .position(|word| word == "link/ether")
                .and_then(|index| line.split_whitespace().nth(index + 1))
        })
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| value != "00:00:00:00:00:00")
}

pub fn current_state(route: &str, wifi: &str) -> NetworkState {
    let (transport, iface) = parse_route(route);
    let (ssid, bssid, unknown) = parse_wifi_status(wifi);
    NetworkState {
        transport,
        iface,
        ssid,
        bssid,
        mac: None,
        validated: if unknown { None } else { Some(true) },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Start,
    Stop,
    Keep,
}

pub fn decide(state: &NetworkState, settings: &Value) -> Decision {
    let enabled = settings
        .get("enable_network_service_control")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    if !enabled {
        return Decision::Keep;
    }
    if state.validated.is_none() {
        return Decision::Keep;
    }
    let on_wifi = state.transport == Transport::Wifi;
    let start_on_wifi = settings
        .get("use_module_on_wifi")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    let stop_on_disconnect = settings
        .get("use_module_on_wifi_disconnect")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    if on_wifi && start_on_wifi {
        Decision::Start
    } else if !on_wifi && stop_on_disconnect {
        Decision::Stop
    } else {
        Decision::Keep
    }
}

pub fn read_current() -> NetworkState {
    let route = Command::new("ip")
        .args(["route"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let wifi = Command::new("cmd")
        .args(["wifi", "status"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let mut state = current_state(&route, &wifi);
    if state.iface.is_none() {
        if let Some(iface) = parse_wifi_iface(&wifi) {
            state.transport = transport_for_iface(&iface);
            state.iface = Some(iface);
        } else {
            let table = Command::new("ip")
                .args(["route", "show", "table", "all"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                .unwrap_or_default();
            if let Some((transport, iface)) = parse_route_table(&table) {
                state.transport = transport;
                state.iface = iface;
            } else {
                let route_get = Command::new("ip")
                    .args(["route", "get", "1.1.1.1"])
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                    .unwrap_or_default();
                if let Some((transport, iface)) = parse_route_get(&route_get) {
                    if iface.as_deref() != Some("meta") && iface.as_deref() != Some("dummy0") {
                        state.transport = transport;
                        state.iface = iface;
                    }
                }
            }
        }
    }
    if let Some(iface) = state.iface.clone() {
        let link = Command::new("ip")
            .args(["link", "show", iface.as_str()])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
            .unwrap_or_default();
        state.mac = parse_mac(&link);
    }
    state
}

pub fn snapshot(settings: &Value) -> Value {
    let state = read_current();
    let decision = decide(&state, settings);
    json!({
        "state": state.json(),
        "decision": match decision {
            Decision::Start => "start",
            Decision::Stop => "stop",
            Decision::Keep => "keep"
        }
    })
}

pub fn should_stop(state: &NetworkState, settings: &Value, running: bool) -> bool {
    if !running {
        return false;
    }
    let enabled = settings
        .get("enable_network_service_control")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    if !enabled || state.validated.is_none() {
        return false;
    }
    let stop_on_disconnect = settings
        .get("use_module_on_wifi_disconnect")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    stop_on_disconnect && state.transport != Transport::Wifi
}

pub fn allowed_mac(state: &NetworkState, settings: &Value) -> bool {
    let macs = settings
        .get("macs_list")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if macs.is_empty() {
        return true;
    }
    let mode = settings
        .get("mac_mode")
        .and_then(Value::as_str)
        .unwrap_or("blacklist");
    let matched = state
        .mac
        .as_deref()
        .is_some_and(|value| macs.iter().any(|entry| entry == value));
    if mode == "whitelist" {
        matched
    } else {
        !matched
    }
}

pub fn allowed_network(state: &NetworkState, settings: &Value) -> bool {
    if !allowed_mac(state, settings) {
        return false;
    }
    let matching = settings
        .get("use_ssid_matching")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    if !matching {
        return true;
    }
    let mode = settings
        .get("use_wifi_list_mode")
        .and_then(Value::as_str)
        .unwrap_or("blacklist");
    let list = |key: &str| {
        settings
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let ssids = list("wifi_ssids_list");
    let bssids = list("wifi_bssids_list");
    if ssids.is_empty() && bssids.is_empty() {
        return true;
    }
    let ssid_match = state
        .ssid
        .as_deref()
        .is_some_and(|value| ssids.iter().any(|entry| entry == value));
    let bssid_match = state
        .bssid
        .as_deref()
        .is_some_and(|value| bssids.iter().any(|entry| entry == value));
    let matched = ssid_match || bssid_match;
    if mode == "whitelist" {
        matched
    } else {
        !matched
    }
}

pub fn should_start(state: &NetworkState, settings: &Value, event_stop: bool) -> bool {
    if !event_stop || state.validated.is_none() || state.transport != Transport::Wifi {
        return false;
    }
    let enabled = settings
        .get("enable_network_service_control")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    let start_on_wifi = settings
        .get("use_module_on_wifi")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "true");
    enabled && start_on_wifi && allowed_network(state, settings)
}

#[derive(Debug)]
struct EventRetry {
    failures: u32,
    max: u32,
    next_at: u64,
    last_error: Option<String>,
}

impl EventRetry {
    fn new(max: u32) -> Self {
        Self {
            failures: 0,
            max,
            next_at: 0,
            last_error: None,
        }
    }

    fn load(paths: &crate::paths::Paths) -> Self {
        let value = crate::util::read_to_string_optional(&paths.event_state)
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let mut retry = Self::new(5);
        if let Some(value) = value {
            retry.failures = value
                .get("failures")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(5) as u32;
            retry.next_at = value.get("next_at").and_then(Value::as_u64).unwrap_or(0);
            retry.last_error = value
                .get("last_error")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        retry
    }

    fn ready(&self, now: u64) -> bool {
        self.failures < self.max && now >= self.next_at
    }

    fn failed(&mut self, now: u64, error: &str) -> u64 {
        self.failures = self.failures.saturating_add(1);
        let shift = self.failures.saturating_sub(1).min(4);
        let delay = 1u64 << shift;
        self.next_at = now.saturating_add(delay * 1000);
        self.last_error = Some(error.to_string());
        delay
    }

    fn reset(&mut self) {
        self.failures = 0;
        self.next_at = 0;
        self.last_error = None;
    }

    fn save(&self, paths: &crate::paths::Paths) {
        let value = json!({
            "failures": self.failures,
            "max": self.max,
            "next_at": self.next_at,
            "last_error": self.last_error,
            "manual": self.failures >= self.max
        });
        let _ = crate::util::atomic_write(
            &paths.event_state,
            serde_json::to_vec_pretty(&value)
                .unwrap_or_default()
                .as_slice(),
            0o600,
        );
    }
}

pub fn reset_control(paths: &crate::paths::Paths) -> crate::error::Result<Value> {
    crate::util::remove_file_if_exists(&paths.event_state)?;
    Ok(json!({"reset": true, "event_control": control_status(paths)}))
}

pub fn control_status(paths: &crate::paths::Paths) -> Value {
    let retry = EventRetry::load(paths);
    json!({
        "failures": retry.failures,
        "next_at": retry.next_at,
        "last_error": retry.last_error,
        "manual": retry.failures >= retry.max
    })
}

pub fn control_loop(paths: crate::paths::Paths) {
    let mut tracker = EventTracker::new(Duration::from_millis(2500));
    let mut retry = EventRetry::load(&paths);
    loop {
        std::thread::sleep(Duration::from_secs(3));
        let now = crate::util::now_ms();
        if !retry.ready(now) {
            continue;
        }
        let state = read_current();
        if tracker.observe(state, now).is_none() {
            continue;
        }
        let Some(state) = tracker.last().cloned() else {
            continue;
        };
        let Ok(store) = crate::settings::SettingsStore::open(paths.clone()) else {
            continue;
        };
        let Ok(settings) = store.load() else {
            continue;
        };
        let settings = Value::Object(settings);
        let control = crate::service::control(&paths);
        if control["mode"].as_str() != Some("auto") {
            continue;
        }
        let running = crate::service::status(&paths)
            .ok()
            .and_then(|status| status["running"].as_bool())
            .unwrap_or(false);
        if running && should_stop(&state, &settings, running) {
            let _ = crate::task::interrupt_all(&paths);
            match crate::service::stop(&paths) {
                Ok(_) => {
                    let _ = crate::util::atomic_write(&paths.event_stop, b"disconnect\n", 0o600);
                    let _ = crate::service::write_control(&paths, "auto", "stopped");
                    retry.reset();
                    let _ = crate::util::remove_file_if_exists(&paths.event_state);
                }
                Err(error) => {
                    let _ = crate::service::write_control(&paths, "auto", "stopped");
                    retry.failed(now, &error.message);
                    retry.save(&paths);
                }
            }
        } else if !running && should_start(&state, &settings, paths.event_stop.is_file()) {
            match crate::service::start(&paths) {
                Ok(_) => {
                    let _ = crate::util::remove_file_if_exists(&paths.event_stop);
                    let _ = crate::service::write_control(&paths, "auto", "running");
                    retry.reset();
                    let _ = crate::util::remove_file_if_exists(&paths.event_state);
                }
                Err(error) => {
                    let _ = crate::service::write_control(&paths, "auto", "running");
                    retry.failed(now, &error.message);
                    retry.save(&paths);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allowed_network, current_state, decide, parse_route, parse_route_get, parse_wifi_iface,
        parse_wifi_status, should_start, should_stop, Decision, EventTracker, NetworkState,
        Transport,
    };
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn parses_route_transports() {
        assert_eq!(
            parse_route("default via 192.168.1.1 dev wlan0 proto dhcp"),
            (Transport::Wifi, Some("wlan0".to_string()))
        );
        assert_eq!(
            parse_route("default via 10.0.0.1 dev rmnet_data0"),
            (Transport::Cellular, Some("rmnet_data0".to_string()))
        );
        assert_eq!(parse_route(""), (Transport::None, None));
    }

    #[test]
    fn parses_link_mac_and_whitelist_blacklist() {
        let mac = super::parse_mac(
            "2: wlan0: <BROADCAST> link/ether AA:BB:CC:DD:EE:FF brd ff:ff:ff:ff:ff:ff",
        );
        assert_eq!(mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        let state = NetworkState {
            transport: Transport::Wifi,
            iface: Some("wlan0".to_string()),
            ssid: Some("Home".to_string()),
            bssid: None,
            mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
            validated: Some(true),
        };
        assert!(super::allowed_mac(
            &state,
            &json!({"mac_mode":"whitelist","macs_list":["AA:BB:CC:DD:EE:FF"]})
        ));
        assert!(!super::allowed_mac(
            &state,
            &json!({"mac_mode":"blacklist","macs_list":["aa:bb:cc:dd:ee:ff"]})
        ));
        assert!(!super::allowed_mac(
            &state,
            &json!({"mac_mode":"whitelist","macs_list":["11:22:33:44:55:66"]})
        ));
    }

    #[test]
    fn parses_android_route_get_and_wifi_iface() {
        let output = "1.1.1.1 via 192.168.1.1 dev wlan0 src 192.168.1.33 uid 0";
        let (transport, iface) = parse_route_get(output).expect("route get");
        assert_eq!(transport, Transport::Wifi);
        assert_eq!(iface.as_deref(), Some("wlan0"));
        assert_eq!(
            parse_wifi_iface("ClientModeManager{id=1 iface=wlan0 role=ROLE_CLIENT_PRIMARY}")
                .as_deref(),
            Some("wlan0")
        );
        let table = "default dev meta table 2022
default via 192.168.1.3 dev wlan0 table wlan0 proto static
default dev dummy0 table 1003";
        let (transport, iface) = super::parse_route_table(table).expect("table route");
        assert_eq!(transport, Transport::Wifi);
        assert_eq!(iface.as_deref(), Some("wlan0"));
    }

    #[test]
    fn parses_wifi_ssid_and_unknown() {
        let (ssid, bssid, unknown) =
            parse_wifi_status("WifiInfo: SSID: \"Home\" BSSID: aa:bb:cc:dd:ee:ff\n");
        assert_eq!(ssid.as_deref(), Some("Home"));
        assert_eq!(bssid.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert!(!unknown);
        let (ssid, _bssid, unknown) = parse_wifi_status("SSID: <unknown ssid>\n");
        assert!(ssid.is_none());
        assert!(unknown);
    }

    #[test]
    fn tracker_debounces_and_deduplicates() {
        let mut tracker = EventTracker::new(Duration::from_millis(100));
        let state = current_state(
            "default via 192.168.1.1 dev wlan0",
            "SSID: \"Home\" BSSID: aa:bb",
        );
        assert!(tracker.observe(state.clone(), 0).is_none());
        assert!(tracker.observe(state.clone(), 50).is_none());
        assert_eq!(tracker.observe(state.clone(), 100), Some(state.clone()));
        assert!(tracker.observe(state, 500).is_none());
    }

    #[test]
    fn auto_start_requires_event_stop_marker_and_wifi() {
        let wifi = NetworkState {
            transport: Transport::Wifi,
            iface: Some("wlan0".to_string()),
            ssid: Some("Home".to_string()),
            bssid: None,
            mac: None,
            validated: Some(true),
        };
        let settings = json!({
            "enable_network_service_control": "true",
            "use_module_on_wifi": "true"
        });
        assert!(should_start(&wifi, &settings, true));
        assert!(!should_start(&wifi, &settings, false));
        let cellular = NetworkState {
            transport: Transport::Cellular,
            ..wifi.clone()
        };
        assert!(!should_start(&cellular, &settings, true));
    }

    #[test]
    fn ssid_lists_filter_auto_start() {
        let home = NetworkState {
            transport: Transport::Wifi,
            iface: Some("wlan0".to_string()),
            ssid: Some("Home".to_string()),
            bssid: Some("aa:bb".to_string()),
            mac: None,
            validated: Some(true),
        };
        let whitelist = json!({
            "use_ssid_matching": "true",
            "use_wifi_list_mode": "whitelist",
            "wifi_ssids_list": ["Home"]
        });
        assert!(allowed_network(&home, &whitelist));
        assert!(!allowed_network(
            &home,
            &json!({
                "use_ssid_matching": "true",
                "use_wifi_list_mode": "whitelist",
                "wifi_ssids_list": ["Office"]
            })
        ));
        assert!(!allowed_network(
            &home,
            &json!({
                "use_ssid_matching": "true",
                "use_wifi_list_mode": "blacklist",
                "wifi_ssids_list": ["Home"]
            })
        ));
        assert!(allowed_network(
            &home,
            &json!({"use_ssid_matching": "false"})
        ));
    }

    #[test]
    fn stop_policy_respects_unknown_and_disabled_control() {
        let state = NetworkState {
            transport: Transport::Cellular,
            iface: Some("rmnet0".to_string()),
            ssid: None,
            bssid: None,
            mac: None,
            validated: Some(true),
        };
        let settings = json!({
            "enable_network_service_control": "true",
            "use_module_on_wifi_disconnect": "true"
        });
        assert!(should_stop(&state, &settings, true));
        assert!(!should_stop(&state, &settings, false));
        let unknown = NetworkState {
            transport: Transport::Cellular,
            iface: None,
            ssid: None,
            bssid: None,
            mac: None,
            validated: None,
        };
        assert!(!should_stop(&unknown, &settings, true));
        let disabled = json!({"enable_network_service_control": "false"});
        assert!(!should_stop(&state, &disabled, true));
    }

    #[test]
    fn event_retry_backs_off_and_stops_at_five_failures() {
        let mut retry = super::EventRetry::new(5);
        assert!(retry.ready(0));
        let delays: Vec<u64> = (0..5)
            .map(|index| retry.failed(index * 1000, "test"))
            .collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16]);
        assert!(!retry.ready(60_000));
        retry.reset();
        assert!(retry.ready(0));
    }

    #[test]
    fn event_retry_state_survives_reload() {
        let root = std::env::temp_dir().join(format!(
            "boxctl-event-retry-{}-{}",
            std::process::id(),
            crate::util::now_ms()
        ));
        let paths = crate::paths::Paths::new(root.clone());
        let mut retry = super::EventRetry::new(5);
        retry.failed(1000, "boom");
        retry.save(&paths);
        let loaded = super::EventRetry::load(&paths);
        assert_eq!(loaded.failures, 1);
        assert_eq!(loaded.last_error.as_deref(), Some("boom"));
        assert!(loaded.next_at > 1000);
        let status = super::control_status(&paths);
        assert_eq!(status["manual"], false);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reset_control_clears_manual_state() {
        let root = std::env::temp_dir().join(format!(
            "boxctl-event-reset-{}-{}",
            std::process::id(),
            crate::util::now_ms()
        ));
        let paths = crate::paths::Paths::new(root.clone());
        let mut retry = super::EventRetry::new(5);
        for index in 0..5 {
            retry.failed(index * 1000, "boom");
        }
        retry.save(&paths);
        assert_eq!(super::control_status(&paths)["manual"], true);
        let result = super::reset_control(&paths).expect("reset");
        assert_eq!(result["event_control"]["failures"], 0);
        assert_eq!(super::control_status(&paths)["manual"], false);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn decisions_keep_unknown_and_respect_manual_off() {
        let unknown = NetworkState {
            transport: Transport::Wifi,
            iface: Some("wlan0".to_string()),
            ssid: None,
            bssid: None,
            mac: None,
            validated: None,
        };
        let settings = json!({
            "enable_network_service_control": "true",
            "use_module_on_wifi": "true",
            "use_module_on_wifi_disconnect": "false"
        });
        assert_eq!(decide(&unknown, &settings), Decision::Keep);
        let disabled = json!({"enable_network_service_control": "false"});
        assert_eq!(decide(&unknown, &disabled), Decision::Keep);
    }
}
