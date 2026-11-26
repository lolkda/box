use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{BoxError, ErrorCode, Result};
use crate::settings::SettingsStore;
use crate::util;

pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
pub struct Policy {
    pub use_ghproxy: bool,
    pub mirror: String,
    pub token: Option<String>,
    pub user_agent: String,
    pub timeout: Duration,
    pub max_bytes: usize,
    pub allow_http: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            use_ghproxy: false,
            mirror: String::new(),
            token: None,
            user_agent: "boxctl".to_string(),
            timeout: Duration::from_secs(30),
            max_bytes: DEFAULT_MAX_BYTES,
            allow_http: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub bytes: usize,
    pub sha256: String,
    pub final_url: String,
}

struct Fetched {
    bytes: Vec<u8>,
    report: Report,
}

pub fn policy_from_settings(store: &SettingsStore) -> Result<Policy> {
    let settings = store.load()?;
    let token = util::read_to_string_optional(&store.paths().token)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(Policy {
        use_ghproxy: settings
            .get("use_ghproxy")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == "true"),
        mirror: settings
            .get("url_ghproxy")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        token,
        user_agent: settings
            .get("subscription_user_agent")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("clash-verge/v2.4.5")
            .to_string(),
        timeout: Duration::from_secs(30),
        max_bytes: DEFAULT_MAX_BYTES,
        allow_http: true,
    })
}

pub fn system_root_certs() -> Option<Vec<ureq::tls::Certificate<'static>>> {
    let dirs = [
        "/system/etc/security/cacerts",
        "/apex/com.android.conscrypt/cacerts",
    ];
    let mut certs = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            if bytes.starts_with(b"-----BEGIN") {
                if let Ok(cert) = ureq::tls::Certificate::from_pem(&bytes) {
                    certs.push(cert);
                }
            } else {
                certs.push(ureq::tls::Certificate::from_der(&bytes).to_owned());
            }
        }
    }
    (!certs.is_empty()).then_some(certs)
}

pub fn download_to_file(
    path: &Path,
    url: &str,
    policy: &Policy,
    expected_sha256: Option<&str>,
) -> Result<Report> {
    let fetched = fetch_body(url, policy, expected_sha256)?;
    util::atomic_write(path, &fetched.bytes, 0o600)?;
    Ok(fetched.report)
}

fn fetch_body(url: &str, policy: &Policy, expected_sha256: Option<&str>) -> Result<Fetched> {
    let (mut current, mirrored) = effective_url(url, policy)?;
    let roots = system_root_certs()
        .map(ureq::tls::RootCerts::from)
        .unwrap_or(ureq::tls::RootCerts::WebPki);
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(policy.timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build())
        .build();
    let agent: ureq::Agent = config.into();

    for _ in 0..=MAX_REDIRECTS {
        let host = host_of(&current)?;
        let send_token = !mirrored
            && policy
                .token
                .as_deref()
                .is_some_and(|token| !token.is_empty())
            && token_allowed_host(&host);
        let mut request = agent.get(&current).header("User-Agent", &policy.user_agent);
        if send_token {
            if let Some(token) = &policy.token {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
        }
        let response = request.call().map_err(map_error)?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    BoxError::new(ErrorCode::Download, "redirect response has no Location")
                })?;
            let base = Url::parse(&current)
                .map_err(|_| BoxError::new(ErrorCode::Download, "invalid redirect base URL"))?;
            current = base
                .join(location)
                .map_err(|_| BoxError::new(ErrorCode::Download, "invalid redirect target"))?
                .to_string();
            validate_download_url(&current, policy.allow_http)?;
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(BoxError::new(
                ErrorCode::Download,
                format!("HTTP status {status}"),
            ));
        }
        if let Some(length) = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
        {
            if length > policy.max_bytes as u64 {
                return Err(BoxError::new(
                    ErrorCode::Content,
                    "download is larger than the configured limit",
                ));
            }
        }
        let mut body = response;
        let bytes = body
            .body_mut()
            .with_config()
            .limit(policy.max_bytes as u64)
            .read_to_vec()
            .map_err(map_error)?;
        if bytes.is_empty() {
            return Err(BoxError::new(ErrorCode::Content, "download body is empty"));
        }
        let digest = Sha256::digest(&bytes);
        let sha256 = format!("{digest:x}");
        if let Some(expected) = expected_sha256 {
            if !expected.eq_ignore_ascii_case(&sha256) {
                return Err(BoxError::new(
                    ErrorCode::Checksum,
                    "download SHA-256 does not match",
                ));
            }
        }
        return Ok(Fetched {
            report: Report {
                bytes: bytes.len(),
                sha256,
                final_url: current,
            },
            bytes,
        });
    }
    Err(BoxError::new(ErrorCode::Download, "too many redirects"))
}

pub fn effective_url(url: &str, policy: &Policy) -> Result<(String, bool)> {
    validate_download_url(url, policy.allow_http)?;
    if !policy.use_ghproxy {
        return Ok((url.to_string(), false));
    }
    let parsed = Url::parse(url)
        .map_err(|_| BoxError::new(ErrorCode::Validation, "invalid download URL"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| BoxError::new(ErrorCode::Validation, "download URL has no host"))?;
    if !matches!(
        host,
        "github.com"
            | "raw.githubusercontent.com"
            | "gist.github.com"
            | "gist.githubusercontent.com"
    ) {
        return Ok((url.to_string(), false));
    }
    let mirror = policy.mirror.trim_end_matches('/');
    if mirror.is_empty() {
        return Err(BoxError::new(
            ErrorCode::Validation,
            "use_ghproxy is enabled but url_ghproxy is empty",
        ));
    }
    validate_download_url(mirror, policy.allow_http)?;
    Ok((format!("{mirror}/{url}"), true))
}

pub fn token_allowed_host(host: &str) -> bool {
    matches!(
        host,
        "github.com"
            | "api.github.com"
            | "raw.githubusercontent.com"
            | "codeload.github.com"
            | "gist.github.com"
    )
}

fn host_of(url: &str) -> Result<String> {
    Url::parse(url)
        .map_err(|_| BoxError::new(ErrorCode::Validation, "invalid download URL"))?
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| BoxError::new(ErrorCode::Validation, "download URL has no host"))
}

fn validate_download_url(url: &str, allow_http: bool) -> Result<()> {
    if allow_http {
        let parsed = Url::parse(url)
            .map_err(|_| BoxError::new(ErrorCode::Validation, "invalid download URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(BoxError::new(
                ErrorCode::Validation,
                "download URL must use http or https",
            ));
        }
        return Ok(());
    }
    validate_https(url)
}

fn validate_https(url: &str) -> Result<()> {
    let parsed = Url::parse(url)
        .map_err(|_| BoxError::new(ErrorCode::Validation, "invalid download URL"))?;
    if parsed.scheme() != "https" {
        return Err(BoxError::new(ErrorCode::Tls, "download URL must use https"));
    }
    if parsed.host_str().is_none() {
        return Err(BoxError::new(
            ErrorCode::Validation,
            "download URL has no host",
        ));
    }
    Ok(())
}

fn map_error(error: ureq::Error) -> BoxError {
    match error {
        ureq::Error::Timeout(_) => BoxError::new(ErrorCode::Timeout, "download timed out"),
        ureq::Error::Tls(message) => {
            BoxError::new(ErrorCode::Tls, format!("TLS failure: {message}"))
        }
        ureq::Error::Rustls(error) => {
            BoxError::new(ErrorCode::Tls, format!("TLS failure: {error}"))
        }
        ureq::Error::StatusCode(code) => {
            BoxError::new(ErrorCode::Download, format!("HTTP status {code}"))
        }
        ureq::Error::Io(error) => BoxError::new(
            ErrorCode::Download,
            format!("download I/O failure: {error}"),
        ),
        other => BoxError::new(ErrorCode::Download, format!("download failed: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_url, token_allowed_host, validate_https, Policy};

    #[test]
    fn rejects_plain_http() {
        let error = validate_https("http://example.com/file").expect_err("http");
        assert_eq!(error.code, crate::error::ErrorCode::Tls);
    }

    #[test]
    fn mirrors_only_github_hosts() {
        let policy = Policy {
            use_ghproxy: true,
            mirror: "https://ghfast.top".to_string(),
            ..Policy::default()
        };
        let (url, mirrored) =
            effective_url("https://github.com/o/r/releases/x", &policy).expect("mirror");
        assert!(mirrored);
        assert_eq!(url, "https://ghfast.top/https://github.com/o/r/releases/x");
        let (url, mirrored) = effective_url("https://example.com/file", &policy).expect("direct");
        assert!(!mirrored);
        assert_eq!(url, "https://example.com/file");
    }

    #[test]
    fn token_hosts_are_allowlisted() {
        assert!(token_allowed_host("github.com"));
        assert!(token_allowed_host("api.github.com"));
        assert!(!token_allowed_host("example.com"));
        assert!(!token_allowed_host("github.com.example.com"));
    }
}
