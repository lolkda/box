use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{BoxError, ErrorCode, Result};
use crate::settings::SettingsStore;
use crate::util;

pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const NET_ADMIN_GID: u32 = 3005;

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
    let fetched = fetch_body_with_egress(url, policy, expected_sha256)?;
    util::atomic_write(path, &fetched.bytes, 0o600)?;
    Ok(fetched.report)
}

/// Q10 download egress policy:
/// When running as root on Linux/Android, try a direct egress attempt first under
/// `gid=3005 (net_admin)` + real DNS (bypassing transparent proxy and fake-ip DNS)
/// so broken proxy nodes never deadlock subscription updates. If direct fails,
/// automatically fall back to system default egress (which goes through the running proxy).
fn fetch_body_with_egress(
    url: &str,
    policy: &Policy,
    expected_sha256: Option<&str>,
) -> Result<Fetched> {
    let is_loopback = Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "::1");
    let is_root = unsafe { libc::geteuid() == 0 };
    if is_root && !is_loopback {
        // Attempt direct bypass (thread-local setegid(3005) via SYS_setresgid on Linux so
        // only this thread's socket creation carries gid=3005, restored immediately after).
        if let Ok(fetched) = run_with_thread_gid(NET_ADMIN_GID, || {
            fetch_body(url, policy, expected_sha256, true)
        }) {
            return Ok(fetched);
        }
    }
    fetch_body(url, policy, expected_sha256, false)
}

fn run_with_thread_gid<T>(gid: u32, f: impl FnOnce() -> T) -> T {
    let orig = unsafe { libc::getegid() };
    // Linux raw syscall SYS_setresgid modifies ONLY the calling thread's credentials
    // (unlike glibc's setegid which broadcasts to all threads).
    let switched = unsafe {
        libc::syscall(
            libc::SYS_setresgid,
            -1 as libc::c_long,
            gid as libc::c_long,
            -1 as libc::c_long,
        ) == 0
    };
    let result = f();
    if switched {
        unsafe {
            let _ = libc::syscall(
                libc::SYS_setresgid,
                -1 as libc::c_long,
                orig as libc::c_long,
                -1 as libc::c_long,
            );
        }
    }
    result
}

#[derive(Debug)]
struct DirectDnsResolver;

impl ureq::unversioned::resolver::Resolver for DirectDnsResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ArrayVec<SocketAddr, 16>, ureq::Error> {
        let host = uri.host().unwrap_or_default();
        let port = uri.port_u16().unwrap_or_else(|| {
            if uri.scheme_str() == Some("http") {
                80
            } else {
                443
            }
        });
        if let Ok(ip) = host.trim_matches(|c| c == '[' || c == ']').parse::<IpAddr>() {
            let mut addrs = ureq::unversioned::resolver::ArrayVec::from_fn(|_| {
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
            });
            addrs.truncate(0);
            addrs.push(SocketAddr::new(ip, port));
            return Ok(addrs);
        }
        // Query real upstream DNS (223.5.5.5, then 1.1.1.1) directly over UDP 53 to bypass fake-ip.
        for server in ["223.5.5.5:53", "1.1.1.1:53"] {
            if let Some(ips) = udp_dns_query_a(host, server) {
                let mut addrs = ureq::unversioned::resolver::ArrayVec::from_fn(|_| {
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
                });
                addrs.truncate(0);
                for ip in ips.into_iter().take(16) {
                    addrs.push(SocketAddr::new(IpAddr::V4(ip), port));
                }
                if !addrs.is_empty() {
                    return Ok(addrs);
                }
            }
        }
        ureq::unversioned::resolver::DefaultResolver::default().resolve(uri, config, timeout)
    }
}

fn udp_dns_query_a(host: &str, server: &str) -> Option<Vec<Ipv4Addr>> {
    let mut packet = Vec::with_capacity(64);
    let txid: u16 = (util::now_ms() & 0xffff) as u16;
    packet.extend_from_slice(&txid.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // QTYPE=A, QCLASS=IN

    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    let _ = socket.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = socket.set_write_timeout(Some(Duration::from_secs(3)));
    socket.send_to(&packet, server).ok()?;
    let mut buf = [0u8; 512];
    let n = socket.recv(&mut buf).ok()?;
    if n < 12 || buf[0] != (txid >> 8) as u8 || buf[1] != (txid & 0xff) as u8 {
        return None;
    }
    let qdcount = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let ancount = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    let mut pos = 12usize;
    for _ in 0..qdcount {
        pos = skip_dns_name(&buf[..n], pos)?;
        pos = pos.checked_add(4)?;
        if pos > n {
            return None;
        }
    }
    let mut results = Vec::new();
    for _ in 0..ancount {
        pos = skip_dns_name(&buf[..n], pos)?;
        if pos + 10 > n {
            break;
        }
        let rtype = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let rdlen = u16::from_be_bytes([buf[pos + 8], buf[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlen > n {
            break;
        }
        if rtype == 1 && rdlen == 4 {
            let ip = Ipv4Addr::new(buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]);
            // Skip fake-ip 198.18.0.0/15 just in case.
            if !(ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19)) {
                results.push(ip);
            }
        }
        pos += rdlen;
    }
    (!results.is_empty()).then_some(results)
}

fn skip_dns_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    let mut jumped = 0usize;
    while pos < buf.len() {
        let len = buf[pos];
        if len == 0 {
            return Some(pos + 1);
        }
        if (len & 0xC0) == 0xC0 {
            return pos.checked_add(2);
        }
        pos = pos.checked_add(1 + len as usize)?;
        jumped += 1;
        if jumped > 128 {
            return None;
        }
    }
    None
}

fn fetch_body(
    url: &str,
    policy: &Policy,
    expected_sha256: Option<&str>,
    direct_dns: bool,
) -> Result<Fetched> {
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
    let agent: ureq::Agent = if direct_dns {
        ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::default(),
            DirectDnsResolver,
        )
    } else {
        config.into()
    };

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
