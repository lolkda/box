use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{BoxError, ErrorCode, Result};
use crate::paths::Paths;
use crate::util;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmParams {
    pub confirm: bool,
}

pub fn status(paths: &Paths) -> Result<Value> {
    let Some(text) = util::read_to_string_optional(&paths.upgrade_state)? else {
        return Ok(defaults());
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| BoxError::new(ErrorCode::Io, "upgrade state is invalid"))?;
    Ok(json!({
        "required": value.get("required").and_then(Value::as_bool).unwrap_or(false),
        "confirmed": value.get("confirmed").and_then(Value::as_bool).unwrap_or(true),
        "from": value.get("from").cloned().unwrap_or(Value::Null),
        "to": value.get("to").cloned().unwrap_or(Value::Null)
    }))
}

pub fn confirm(paths: &Paths, params: ConfirmParams) -> Result<Value> {
    if !params.confirm {
        return Err(BoxError::new(
            ErrorCode::NeedsConfirmation,
            "upgrade confirmation requires confirm=true",
        ));
    }
    let current = status(paths)?;
    if !current["required"].as_bool().unwrap_or(false) {
        return Ok(current);
    }
    let data = json!({
        "required": false,
        "confirmed": true,
        "from": current["from"].clone(),
        "to": current["to"].clone()
    });
    util::ensure_dir(
        paths
            .upgrade_state
            .parent()
            .ok_or_else(|| BoxError::internal("upgrade state has no parent"))?,
        true,
    )?;
    util::atomic_write(
        &paths.upgrade_state,
        serde_json::to_vec_pretty(&data)
            .map_err(|_| BoxError::internal("cannot serialize upgrade state"))?
            .as_slice(),
        0o600,
    )?;
    Ok(data)
}

fn defaults() -> Value {
    json!({
        "required": false,
        "confirmed": true,
        "from": Value::Null,
        "to": Value::Null
    })
}

#[cfg(test)]
mod tests {
    use super::{confirm, status, ConfirmParams};
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let root =
            std::env::temp_dir().join(format!("boxctl-upgrade-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("state")).expect("state");
        root
    }

    #[test]
    fn confirm_requires_pending_upgrade_and_explicit_flag() {
        let root = root();
        let paths = Paths::new(root.clone());
        let idle = status(&paths).expect("idle");
        assert_eq!(idle["required"], false);
        fs::write(
            root.join("state/upgrade.json"),
            r#"{"required":true,"confirmed":false,"from":"1.4.0","to":"2.0.0"}"#,
        )
        .expect("state");
        let pending = status(&paths).expect("pending");
        assert_eq!(pending["required"], true);
        assert_eq!(pending["confirmed"], false);
        let error = confirm(&paths, ConfirmParams { confirm: false }).expect_err("confirm");
        assert_eq!(error.code, crate::error::ErrorCode::NeedsConfirmation);
        let done = confirm(&paths, ConfirmParams { confirm: true }).expect("confirm");
        assert_eq!(done["required"], false);
        assert_eq!(done["confirmed"], true);
        let again = confirm(&paths, ConfirmParams { confirm: true }).expect("idempotent");
        assert_eq!(again["required"], false);
        let _ = fs::remove_dir_all(root);
    }
}
