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
        let mut candidate = (unix_seconds / MINUTE + 1) * MINUTE;
        for _ in 0..MAX_SCAN_MINUTES {
            let days = (candidate / DAY) as i64;
            let seconds_of_day = candidate % DAY;
            let hour = (seconds_of_day / 3600) as u32;
            let minute = ((seconds_of_day % 3600) / 60) as u32;
            let (year, month, dom) = civil_from_days(days);
            let dow = weekday_from_days(days);
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
                let _ = year;
                return Some(candidate);
            }
            candidate += MINUTE;
        }
        None
    }
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

pub fn is_due(scheduler: &Scheduler, state: &ScheduleState, now: u64, grace: u64) -> bool {
    if !scheduler.enabled {
        return false;
    }
    if state.last_run == 0 {
        return true;
    }
    let Some(next) = scheduler.next_after(state.last_run) else {
        return false;
    };
    next <= now && now.saturating_sub(next) <= grace
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
        assert_eq!(cron.next_after(start), Some(start + 300));
        let cron = Cron::parse("30 4 * * 1-5").expect("cron");
        let next = cron.next_after(start).expect("next");
        assert_eq!((next - start) / 60, 270);
        let sunday = Cron::parse("0 0 * * 7").expect("sunday");
        assert!(sunday.next_after(start).is_some());
    }

    #[test]
    fn cron_rejects_bad_shapes_and_ranges() {
        assert!(Cron::parse("* * * *").is_err());
        assert!(Cron::parse("60 * * * *").is_err());
        assert!(Cron::parse("* * 0 * *").is_err());
        assert!(Cron::parse("*/0 * * * *").is_err());
        assert!(Cron::parse("* * * * 8").is_err());
    }

    #[test]
    fn scheduler_defaults_are_disabled_and_tasks_are_allowlisted() {
        let disabled = Scheduler::from_settings(&json!({
            "run_crontab": "false",
            "update_subscription": "true",
            "update_geo": "true",
            "interva_update": "*/5 * * * *"
        }))
        .expect("scheduler");
        assert!(!disabled.enabled);
        assert!(disabled.tasks.is_empty());
        let enabled = Scheduler::from_settings(&json!({
            "run_crontab": "true",
            "update_subscription": "true",
            "update_geo": "false",
            "interva_update": "*/5 * * * *"
        }))
        .expect("scheduler");
        assert_eq!(enabled.tasks, vec!["sub-update"]);
    }

    #[test]
    fn missed_runs_are_bounded_by_grace() {
        let scheduler = Scheduler::from_settings(&json!({
            "run_crontab": "true",
            "update_subscription": "true",
            "update_geo": "false",
            "interva_update": "* * * * *"
        }))
        .expect("scheduler");
        let now = epoch(2026, 1, 1) + 10 * 60;
        let state = super::ScheduleState {
            last_run: epoch(2026, 1, 1),
            last_task: Some("sub-update".to_string()),
        };
        assert!(is_due(&scheduler, &state, now, 3600));
        let old = super::ScheduleState {
            last_run: epoch(2025, 12, 31),
            last_task: None,
        };
        assert!(!is_due(&scheduler, &old, now, 3600));
    }
}
