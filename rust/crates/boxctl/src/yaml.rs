use std::fs;
use std::path::Path;

use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};

use crate::error::{BoxError, ErrorCode, Result};
use crate::util;

pub const CONFIG_MODE: u32 = 0o600;

pub fn read_raw(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            BoxError::new(ErrorCode::NotFound, "file does not exist")
                .with_detail(path.display().to_string())
        } else {
            BoxError::from(error).with_detail(format!("cannot read {}", path.display()))
        }
    })
}

pub fn read_map(path: &Path) -> Result<Mapping> {
    let raw = read_raw(path)?;
    let value: YamlValue = serde_yaml::from_str(&raw).map_err(|error| {
        BoxError::validation(format!("{} is not valid YAML", path.display()))
            .with_detail(error.to_string())
    })?;
    match value {
        YamlValue::Mapping(map) => Ok(map),
        _ => Err(BoxError::validation(format!(
            "{} must contain a YAML mapping",
            path.display()
        ))),
    }
}

pub fn write_map(path: &Path, map: &Mapping) -> Result<()> {
    let text = serde_yaml::to_string(&YamlValue::Mapping(map.clone())).map_err(|error| {
        BoxError::internal("cannot serialize YAML").with_detail(error.to_string())
    })?;
    validate_mapping(&text, path)?;
    util::atomic_write(path, text.as_bytes(), CONFIG_MODE)
}

pub fn write_raw(path: &Path, raw: &str) -> Result<()> {
    validate_mapping(raw, path)?;
    util::atomic_write(path, raw.as_bytes(), CONFIG_MODE)
}

pub fn validate_mapping(raw: &str, path: &Path) -> Result<()> {
    let value: YamlValue = serde_yaml::from_str(raw).map_err(|error| {
        BoxError::validation("YAML syntax is invalid").with_detail(error.to_string())
    })?;
    if !value.is_mapping() {
        return Err(BoxError::validation(format!(
            "{} must contain a YAML mapping",
            path.display()
        )));
    }
    Ok(())
}

pub fn to_json(value: &YamlValue) -> Result<JsonValue> {
    serde_json::to_value(value).map_err(|error| {
        BoxError::internal("cannot convert YAML value").with_detail(error.to_string())
    })
}

pub fn from_json(value: &JsonValue) -> Result<YamlValue> {
    serde_yaml::to_value(value).map_err(|error| {
        BoxError::validation("cannot convert JSON value to YAML").with_detail(error.to_string())
    })
}
