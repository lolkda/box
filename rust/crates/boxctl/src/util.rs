use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::error::{BoxError, ErrorCode, Result};

const DIR_MODE: u32 = 0o700;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

pub fn ensure_dir(path: &Path, private: bool) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(BoxError::usage("empty directory path"));
    }
    if path.exists() {
        if !path.is_dir() {
            return Err(BoxError::new(ErrorCode::Io, "path is not a directory")
                .with_detail(path.display().to_string()));
        }
    } else {
        fs::create_dir_all(path).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot create {}", path.display()))
        })?;
    }
    if private {
        fs::set_permissions(path, fs::Permissions::from_mode(DIR_MODE)).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot chmod {}", path.display()))
        })?;
    }
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| BoxError::internal("target has no parent"))?;
    ensure_dir(parent, false)?;

    let temp = unique_temp_path(path);
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)
            .map_err(|error| {
                BoxError::from(error).with_detail(format!("cannot create {}", temp.display()))
            })?;
        file.write_all(bytes).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot write {}", temp.display()))
        })?;
        file.sync_all().map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot sync {}", temp.display()))
        })?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode)).map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot chmod {}", temp.display()))
        })?;
        drop(file);
        fs::rename(&temp, path).map_err(|error| {
            BoxError::from(error).with_detail(format!(
                "cannot replace {} with {}",
                path.display(),
                temp.display()
            ))
        })?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map_or_else(
        || "boxctl".into(),
        |value| value.to_string_lossy().into_owned(),
    );
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.tmp.{}.{}", std::process::id(), counter))
}

pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(BoxError::from(error).with_detail(format!("cannot read {}", path.display())))
        }
    }
}

pub fn read_to_string_optional(path: &Path) -> Result<Option<String>> {
    read_optional(path)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                BoxError::new(ErrorCode::Validation, "file is not valid UTF-8")
                    .with_detail(path.display().to_string())
            })
        })
        .transpose()
}

pub fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    let bytes = fs::read(path).map_err(|error| {
        BoxError::from(error).with_detail(format!("cannot read {}", path.display()))
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        BoxError::validation(format!("{} is not valid JSON", path.display()))
            .with_detail(error.to_string())
    })?;
    value.as_object().cloned().ok_or_else(|| {
        BoxError::validation(format!("{} must contain a JSON object", path.display()))
    })
}

pub fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(BoxError::from(error).with_detail(format!("cannot remove {}", path.display())))
        }
    }
}

#[cfg(test)]
pub fn file_mode(path: &Path) -> Result<u32> {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .map_err(|error| {
            BoxError::from(error).with_detail(format!("cannot stat {}", path.display()))
        })
}

pub fn format_unix_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = rest / 3_600;
    let minute = (rest % 3_600) / 60;
    let second = rest % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

pub fn write_stdout(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::{atomic_write, ensure_dir, file_mode};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let path =
            std::env::temp_dir().join(format!("boxctl-util-{}-{suffix}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn writes_private_files_atomically() {
        let root = temp_dir();
        let target = root.join("settings.json");
        atomic_write(&target, b"{}\n", 0o600).expect("write");
        assert_eq!(fs::read(&target).expect("read"), b"{}\n");
        assert_eq!(file_mode(&target).expect("mode"), 0o600);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creates_private_directory() {
        let root = temp_dir();
        let nested = root.join("run").join("boxctl");
        ensure_dir(&nested, true).expect("mkdir");
        assert!(nested.is_dir());
        assert_eq!(file_mode(&nested).expect("mode"), 0o700);
        let _ = fs::remove_dir_all(root);
    }
}
