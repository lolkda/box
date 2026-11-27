use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;

const MINUTE: u64 = 60;
const DAY: u64 = 24 * 60 * 60;
const MAX_SCAN_MINUTES: u64 = 5 * 366 * 24 * 60;

#[derive(Debug, Clone)]
pub struct Cron {
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    dow: Field,
}

#[derive(Debug, Clone)]
struct Field {
    allowed: Vec<bool>,
    all: bool,
}

impl Field {
    fn parse(value: &str, min: u32, max: u32, names: &[(&str, u32)]) -> Result<Self> {
        let mut allowed = vec![false; (max - min + 1) as usize];
        let mut all = false;
        for part in value.split(',') {
            if part.is_empty() {
                return Err(BoxError::validation("cron field contains an empty item"));
            }
            let (range, step) = match part.split_once('/') {
                Some((range, step)) => {
                    let step = step
                        .parse::<u32>()
                        .map_err(|_| BoxError::validation("cron step is not a number"))?;
                    if step == 0 {
                        return Err(BoxError::validation("cron step must be positive"));
                    }
                    (range, step)
                }
                None => (part, 1),
            };
            let (start, end) = if range == "*" {
                all = true;
                (min, max)
            } else if let Some((start, end)) = range.split_once('-') {
                (
                    number(start, min, max, names)?,
                    number(end, min, max, names)?,
                )
            } else {
                let value = number(range, min, max, names)?;
                if part.contains('/') {
                    (value, max)
                } else {
                    (value, value)
                }
            };
            if start > end {
                return Err(BoxError::validation("cron range is reversed"));
            }
            let mut current = start;
            while current <= end {
                allowed[(current - min) as usize] = true;
                current += step;
            }
        }
        if !allowed.iter().any(|value| *value) {
            return Err(BoxError::validation("cron field has no allowed value"));
        }
        Ok(Self { allowed, all })
    }

    fn matches(&self, value: u32, min: u32) -> bool {
        self.allowed
            .get((value - min) as usize)
            .copied()
            .unwrap_or(false)
    }
}

impl Cron {
    pub fn parse(expression: &str) -> Result<Self> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(BoxError::validation("cron must have exactly 5 fields"));
        }
        let month_names = [
            ("jan", 1),
            ("feb", 2),
            ("mar", 3),
            ("apr", 4),
            ("may", 5),
            ("jun", 6),
            ("jul", 7),
            ("aug", 8),
            ("sep", 9),
            ("oct", 10),
            ("nov", 11),
            ("dec", 12),
        ];
        let dow_names = [
            ("sun", 0),
            ("mon", 1),
            ("tue", 2),
            ("wed", 3),
            ("thu", 4),
            ("fri", 5),
            ("sat", 6),
        ];
        Ok(Self {
            minute: Field::parse(fields[0], 0, 59, &[])?,
            hour: Field::parse(fields[1], 0, 23, &[])?,
            dom: Field::parse(fields[2], 1, 31, &[])?,
            month: Field::parse(fields[3], 1, 12, &month_names)?,
            dow: {
                let mut field = Field::parse(fields[4], 0, 7, &dow_names)?;
                if field.allowed.get(7).copied().unwrap_or(false) {
                    field.allowed[0] = true;
                }
                field
            },
        })
    }

    pub fn next_after(&self, unix_seconds: u64) -> Option<u64> {
        self.next_after_mode(unix_seconds, true)
    }

    #[cfg(test)]
    pub fn next_after_utc(&self, unix_seconds: u64) -> Option<u64> {
        self.next_after_mode(unix_seconds, false)
    }

    fn next_after_mode(&self, unix_seconds: u64, local_tz: bool) -> Option<u64> {
        let mut candidate = (unix_seconds / MINUTE + 1) * MINUTE;
        for _ in 0..MAX_SCAN_MINUTES {
            let (month, dom, dow, hour, minute) = if local_tz {
                local_parts(candidate)
            } else {
                utc_parts(candidate)
            };
            let dom_restricted = !self.dom.all;
            let dow_restricted = !self.dow.all;
            let day_ok = if dom_restricted && dow_restricted {
                self.dom.matches(dom, 1) || self.dow.matches(dow, 0)
            } else {
                self.dom.matches(dom, 1) && self.dow.matches(dow, 0)
            };
            if self.month.matches(month, 1)
                && day_ok
                && self.hour.matches(hour, 0)
                && self.minute.matches(minute, 0)
            {
                return Some(candidate);
            }
            candidate += MINUTE;
        }
        None
    }
}

fn utc_parts(unix_seconds: u64) -> (u32, u32, u32, u32, u32) {
    let days = (unix_seconds / DAY) as i64;
    let seconds_of_day = unix_seconds % DAY;
    let hour = (seconds_of_day / 3600) as u32;
    let minute = ((seconds_of_day % 3600) / 60) as u32;
    let (_year, month, dom) = civil_from_days(days);
    let dow = weekday_from_days(days);
    (month, dom, dow, hour, minute)
}

fn local_parts(unix_seconds: u64) -> (u32, u32, u32, u32, u32) {
    let ts = unix_seconds as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ptr = unsafe { libc::localtime_r(&ts, &mut tm) };
    if ptr.is_null() {
        return utc_parts(unix_seconds);
    }
    let month = (tm.tm_mon + 1).max(1) as u32;
    let dom = tm.tm_mday.max(1) as u32;
    let dow = tm.tm_wday.rem_euclid(7) as u32;
    let hour = tm.tm_hour.clamp(0, 23) as u32;
    let minute = tm.tm_min.clamp(0, 59) as u32;
    (month, dom, dow, hour, minute)
}

fn number(value: &str, min: u32, max: u32, names: &[(&str, u32)]) -> Result<u32> {
    let normalized = value.to_ascii_lowercase();
    let number = if let Ok(number) = normalized.parse::<u32>() {
        number
    } else {
        names
            .iter()
            .find(|(name, _)| *name == normalized)
            .map(|(_, value)| *value)
            .ok_or_else(|| BoxError::validation(format!("invalid cron value: {value}")))?
    };
    if number < min || number > max {
        return Err(BoxError::validation(format!(
            "cron value out of range: {value}"
        )));
    }
    Ok(number)
}

fn weekday_from_days(days: i64) -> u32 {
    (((days + 4) % 7 + 7) % 7) as u32
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i64::from(m <= 2), m as u32, d as u32)
}

#[cfg(test)]
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(month) + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    pub enabled: bool,
    pub cron: Option<Cron>,
    pub tasks: Vec<String>,
}

impl Scheduler {
    pub fn from_settings(settings: &Value) -> Result<Self> {
        let enabled = settings
            .get("run_crontab")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "true");
        let expression = settings
            .get("interva_update")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let cron = if expression.is_empty() {
            None
        } else {
            Some(Cron::parse(&expression)?)
        };
        let mut tasks = Vec::new();
        if enabled {
            if settings
                .get("update_subscription")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "true")
            {
                tasks.push("sub-update".to_string());
            }
            if settings
                .get("update_geo")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "true")
            {
                tasks.push("geox".to_string());
            }
        }
        Ok(Self {
            enabled,
            cron,
            tasks,
        })
    }

    pub fn next_after(&self, now: u64) -> Option<u64> {
        self.cron.as_ref().and_then(|cron| cron.next_after(now))
    }
}

#[derive(Debug, Clone)]
pub struct ScheduleState {
    pub last_run: u64,
    pub last_task: Option<String>,
}

pub fn state(paths: &Paths) -> Result<ScheduleState> {
    let Some(value) = util::read_to_string_optional(&paths.schedule_state)? else {
        return Ok(ScheduleState {
            last_run: 0,
            last_task: None,
        });
    };
    let parsed: Value = serde_json::from_str(&value)
        .map_err(|_| BoxError::new(ErrorCode::Io, "schedule state is invalid"))?;
    Ok(ScheduleState {
        last_run: parsed.get("last_run").and_then(Value::as_u64).unwrap_or(0),
        last_task: parsed
            .get("last_task")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub fn status(paths: &Paths, settings: &Value, now: u64) -> Result<Value> {
    let scheduler = Scheduler::from_settings(settings)?;
    let state = state(paths)?;
    Ok(json!({
        "enabled": scheduler.enabled,
        "cron": scheduler.cron.is_some(),
        "tasks": scheduler.tasks,
        "last_run": state.last_run,
        "last_task": state.last_task,
        "next_run": if scheduler.enabled { scheduler.next_after(now) } else { None },
        "due": is_due(&scheduler, &state, now, 3600)
    }))
}

pub fn mark_run(paths: &Paths, task: &str, now: u64) -> Result<()> {
    util::ensure_dir(
        paths
            .schedule_state
            .parent()
            .ok_or_else(|| BoxError::internal("schedule state has no parent"))?,
        true,
    )?;
    let data = json!({"last_run": now, "last_task": task});
    util::atomic_write(
        &paths.schedule_state,
        serde_json::to_vec_pretty(&data)
            .map_err(|_| BoxError::internal("cannot serialize schedule state"))?
            .as_slice(),
        0o600,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DueCheck {
    NotDue,
    Due,
    /// Missed window exceeded `grace` or uninitialized baseline (`last_run == 0`);
    /// caller should advance `last_run` to `now` so the scheduler never stalls permanently.
    AdvanceBaseline,
}

pub fn check_due(scheduler: &Scheduler, state: &ScheduleState, now: u64, grace: u64) -> DueCheck {
    if !scheduler.enabled {
        return DueCheck::NotDue;
    }
    if state.last_run == 0 {
        // Do not fire immediately when crontab is first turned on; wait for next scheduled minute.
        return DueCheck::AdvanceBaseline;
    }
    let Some(next) = scheduler.next_after(state.last_run) else {
        return DueCheck::NotDue;
    };
    if next > now {
        DueCheck::NotDue
    } else if now.saturating_sub(next) <= grace {
        DueCheck::Due
    } else {
        DueCheck::AdvanceBaseline
    }
}

pub fn is_due(scheduler: &Scheduler, state: &ScheduleState, now: u64, grace: u64) -> bool {
    check_due(scheduler, state, now, grace) == DueCheck::Due
}

#[derive(Debug, Clone)]
pub struct CrontabEntry {
    pub id: String,
    pub cron: Cron,
    pub command: String,
}

pub fn load_crontab_entries(paths: &Paths) -> Vec<CrontabEntry> {
    let Ok(Some(text)) = util::read_to_string_optional(&paths.crontab) else {
        return Vec::new();
    };
    parse_crontab_entries(&text)
}

pub fn parse_crontab_entries(text: &str) -> Vec<CrontabEntry> {
    let mut entries = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let mut fields = Vec::with_capacity(5);
        for _ in 0..5 {
            if let Some(field) = parts.next() {
                fields.push(field);
            }
        }
        if fields.len() < 5 {
            continue;
        }
        let expr = fields.join(" ");
        let command = parts.collect::<Vec<_>>().join(" ");
        if command.is_empty() {
            continue;
        }
        if let Ok(cron) = Cron::parse(&expr) {
            entries.push(CrontabEntry {
                id: format!("crontab-{}", idx + 1),
                cron,
                command,
            });
        }
    }
    entries
}

/// Map legacy `box.tool` commands inside `crontab.cfg` to native boxctl task kinds
/// when possible; returns `None` for arbitrary custom shell commands.
pub fn map_legacy_cron_command(command: &str) -> Option<&'static str> {
    let trimmed = command.trim();
    if !trimmed.contains("box.tool") {
        return None;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let pos = tokens.iter().position(|t| t.ends_with("box.tool"))?;
    let sub = tokens.get(pos + 1).copied().unwrap_or("");
    match sub {
        "subs" | "geosub" => Some("sub-update"),
        "geox" | "upgeox_all" => Some("geox"),
        "upcnip" => Some("upcnip"),
        "upxui" => Some("upxui"),
        "upyq" => Some("upyq"),
        "upcurl" => Some("upcurl"),
        "upkernel" => Some("upkernel"),
        _ => None,
    }
}

pub fn run_crontab_entry(paths: &Paths, entry: &CrontabEntry) {
    if let Some(kind) = map_legacy_cron_command(&entry.command) {
        if crate::task::running_count(paths).unwrap_or(0) == 0 {
            let _ = crate::task::start(
                paths,
                crate::task::StartParams {
                    kind: kind.to_string(),
                    arg: None,
                    wait: true,
                },
            );
        }
        return;
    }
    let _ = std::process::Command::new("/system/bin/sh")
        .arg("-c")
        .arg(&entry.command)
        .env("BOX_HOME", &paths.home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .or_else(|_| {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&entry.command)
                .env("BOX_HOME", &paths.home)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        });
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs())
}

#[cfg(test)]
mod tests {
    use super::{days_from_civil, is_due, Cron, Scheduler};
    use serde_json::json;

    fn epoch(year: i64, month: u32, day: u32) -> u64 {
        (days_from_civil(year, month, day) * 86_400) as u64
    }

    #[test]
    fn cron_parses_and_calculates_next_minute() {
        let cron = Cron::parse("*/5 * * * *").expect("cron");
        let start = epoch(2026, 1, 1);
        assert_eq!(cron.next_after_utc(start), Some(start + 300));
        let cron = Cron::parse("30 4 * * 1-5").expect("cron");
        let next = cron.next_after_utc(start).expect("next");
        assert_eq!((next - start) / 60, 270);
        let sunday = Cron::parse("0 0 * * 7").expect("sunday");
        assert!(sunday.next_after_utc(start).is_some());
    }

    #[test]
    fn missed_runs_advance_baseline_instead_of_stalling_and_zero_does_not_fire_immediately() {
        let scheduler = Scheduler::from_settings(&json!({
            "run_crontab": "true",
            "update_subscription": "true",
            "update_geo": "false",
            "interva_update": "* * * * *"
        }))
        .expect("scheduler");
        let now = epoch(2026, 1, 1) + 10 * 60;
        let fresh = super::ScheduleState {
            last_run: 0,
            last_task: None,
        };
        assert_eq!(
            super::check_due(&scheduler, &fresh, now, 3600),
            super::DueCheck::AdvanceBaseline
        );
        assert!(!is_due(&scheduler, &fresh, now, 3600));
        let state = super::ScheduleState {
            last_run: epoch(2026, 1, 1),
            last_task: Some("sub-update".to_string()),
        };
        assert_eq!(
            super::check_due(&scheduler, &state, now, 3600),
            super::DueCheck::Due
        );
        let old = super::ScheduleState {
            last_run: epoch(2025, 12, 31),
            last_task: None,
        };
        assert_eq!(
            super::check_due(&scheduler, &old, now, 3600),
            super::DueCheck::AdvanceBaseline
        );
    }

    #[test]
    fn crontab_cfg_parses_and_maps_legacy_box_tool_commands() {
        let text = "# comment\n0 3 * * * /data/adb/box/scripts/box.tool geosub\n*/15 * * * * echo hi\n";
        let entries = super::parse_crontab_entries(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            super::map_legacy_cron_command(&entries[0].command),
            Some("sub-update")
        );
        assert_eq!(super::map_legacy_cron_command(&entries[1].command), None);
    }
}
