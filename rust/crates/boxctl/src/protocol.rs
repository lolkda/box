use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BoxError, Result};

pub const CONTRACT_VERSION: u32 = 2;
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1_048_576;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: Option<Value>,
    pub client_version: String,
    pub protocol: u32,
}

impl Request {
    pub fn parse(line: &str) -> Result<Self> {
        serde_json::from_str(line).map_err(|error| {
            BoxError::protocol("request is not valid NDJSON JSON").with_detail(error.to_string())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    pub protocol: u32,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
    pub meta: Meta,
}

impl Response {
    pub fn success(
        data: Value,
        request_id: impl Into<String>,
        duration_ms: u64,
        revision: Option<u64>,
    ) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
            meta: Meta {
                version: CONTRACT_VERSION,
                protocol: PROTOCOL_VERSION,
                request_id: request_id.into(),
                duration_ms: Some(duration_ms),
                revision,
                warnings: Vec::new(),
            },
        }
    }

    pub fn error(error: BoxError, request_id: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(ErrorPayload {
                code: error.code.as_str().to_string(),
                message: error.message,
                detail: error.detail,
                retryable: error.retryable,
            }),
            meta: Meta {
                version: CONTRACT_VERSION,
                protocol: PROTOCOL_VERSION,
                request_id: request_id.into(),
                duration_ms: Some(duration_ms),
                revision: None,
                warnings: Vec::new(),
            },
        }
    }

    pub fn encode(&self) -> Result<String> {
        let mut line = serde_json::to_string(self).map_err(|error| {
            BoxError::internal("cannot encode response").with_detail(error.to_string())
        })?;
        line.push('\n');
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, Response, CONTRACT_VERSION, PROTOCOL_VERSION};
    use crate::error::BoxError;
    use serde_json::json;

    #[test]
    fn request_rejects_unknown_fields() {
        let error = Request::parse(
            r#"{"id":"1","method":"settings.get","params":{},"client_version":"t","protocol":1,"extra":true}"#,
        )
        .expect_err("unknown field must fail");
        assert_eq!(error.code, crate::error::ErrorCode::Protocol);
    }

    #[test]
    fn success_has_v2_meta() {
        let response = Response::success(json!({"ok":true}), "req", 3, Some(7));
        let line = response.encode().expect("encode");
        let value: serde_json::Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(value["meta"]["version"], CONTRACT_VERSION);
        assert_eq!(value["meta"]["protocol"], PROTOCOL_VERSION);
        assert_eq!(value["meta"]["revision"], 7);
    }

    #[test]
    fn error_has_no_data() {
        let response = Response::error(BoxError::validation("bad"), "req", 1);
        let line = response.encode().expect("encode");
        let value: serde_json::Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(value["ok"], false);
        assert!(value.get("data").is_none());
        assert_eq!(value["error"]["code"], "E_VALIDATION");
    }
}
