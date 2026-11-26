use std::fs::OpenOptions;
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{BoxError, Result};
use crate::paths::Paths;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetParams {
    #[serde(default)]
    pub lines: Option<u64>,
}

pub fn get(paths: &Paths, params: GetParams) -> Result<Value> {
    let count = params.lines.unwrap_or(200);
    if count > 10_000 {
        return Err(BoxError::validation("logs lines must be 0..=10000"));
    }
    let token = util::read_to_string_optional(&paths.token)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (source_path, text) = match util::read_to_string_optional(&paths.runs_log)? {
        Some(text) => (&paths.runs_log, text),
        None => match util::read_to_string_optional(&paths.tool_log)? {
            Some(text) => (&paths.tool_log, text),
            None => (&paths.runs_log, String::new()),
        },
    };
    let mut lines = text
        .lines()
        .rev()
        .take(count as usize)
        .map(|line| match &token {
            Some(token) => line.replace(token, "***"),
            None => line.to_string(),
        })
        .collect::<Vec<_>>();
    lines.reverse();
    Ok(json!({
        "file": source_path.display().to_string(),
        "lines": lines
    }))
}

pub fn clear(paths: &Paths) -> Result<Value> {
    let mut cleared = Vec::new();
    for path in [&paths.runs_log, &paths.tool_log, &paths.core_log] {
        if clear_file(path)? {
            cleared.push(path.display().to_string());
        }
    }
    Ok(json!({"cleared": cleared}))
}

fn clear_file(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot clear {}", path.display()))
        })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{clear, get, GetParams};
    use crate::paths::Paths;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-logs-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("run")).expect("run");
        root
    }

    #[test]
    fn redacts_configured_token() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(&paths.token, "secret-token\n").expect("token");
        fs::write(&paths.runs_log, "token=secret-token\n").expect("log");
        let data = get(&paths, GetParams { lines: None }).expect("get");
        assert_eq!(data["lines"], json!(["token=***"]));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn returns_tail_and_clears_existing_logs() {
        let root = temp_root();
        let paths = Paths::new(root.clone());
        fs::write(&paths.runs_log, "a\nb\nc\n").expect("log");
        fs::write(&paths.tool_log, "tool\n").expect("tool");
        let data = get(&paths, GetParams { lines: Some(2) }).expect("get");
        assert_eq!(data["lines"], json!(["b", "c"]));
        let cleared = clear(&paths).expect("clear");
        assert_eq!(cleared["cleared"].as_array().expect("cleared").len(), 2);
        assert_eq!(fs::read(&paths.runs_log).expect("runs"), b"");
        assert_eq!(fs::read(&paths.tool_log).expect("tool"), b"");
        let _ = fs::remove_dir_all(root);
    }
}
