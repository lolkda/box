use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, BoxError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Internal,
    Io,
    Download,
    Tls,
    Usage,
    Validation,
    Content,
    Arch,
    Checksum,
    Permission,
    ManagerUnavailable,
    Protocol,
    Conflict,
    Busy,
    VersionMismatch,
    NeedsConfirmation,
    NotFound,
    Timeout,
    Canceled,
    Interrupted,
    Command,
    Reload,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "E_INTERNAL",
            Self::Io => "E_IO",
            Self::Download => "E_DOWNLOAD",
            Self::Tls => "E_TLS",
            Self::Usage => "E_USAGE",
            Self::Validation => "E_VALIDATION",
            Self::Content => "E_CONTENT",
            Self::Arch => "E_ARCH",
            Self::Checksum => "E_CHECKSUM",
            Self::Permission => "E_PERMISSION",
            Self::ManagerUnavailable => "E_MANAGER_UNAVAILABLE",
            Self::Protocol => "E_PROTOCOL",
            Self::Conflict => "E_CONFLICT",
            Self::Busy => "E_BUSY",
            Self::VersionMismatch => "E_VERSION_MISMATCH",
            Self::NeedsConfirmation => "E_NEEDS_CONFIRMATION",
            Self::NotFound => "E_NOT_FOUND",
            Self::Timeout => "E_TIMEOUT",
            Self::Canceled => "E_CANCELED",
            Self::Interrupted => "E_INTERRUPTED",
            Self::Command => "E_COMMAND",
            Self::Reload => "E_RELOAD",
        }
    }

    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Internal
            | Self::Io
            | Self::Download
            | Self::Tls
            | Self::Command
            | Self::Reload => 1,
            Self::Usage => 2,
            Self::Validation | Self::Content | Self::Arch | Self::Checksum => 3,
            Self::Permission => 4,
            Self::ManagerUnavailable | Self::Protocol => 5,
            Self::Conflict | Self::Busy | Self::VersionMismatch => 6,
            Self::NeedsConfirmation => 7,
            Self::NotFound => 8,
            Self::Timeout => 9,
            Self::Canceled | Self::Interrupted => 10,
        }
    }

    pub const fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::Download | Self::ManagerUnavailable | Self::Busy | Self::Timeout | Self::Reload
        )
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "E_INTERNAL" => Self::Internal,
            "E_IO" => Self::Io,
            "E_DOWNLOAD" => Self::Download,
            "E_TLS" => Self::Tls,
            "E_USAGE" => Self::Usage,
            "E_VALIDATION" => Self::Validation,
            "E_CONTENT" => Self::Content,
            "E_ARCH" => Self::Arch,
            "E_CHECKSUM" => Self::Checksum,
            "E_PERMISSION" => Self::Permission,
            "E_MANAGER_UNAVAILABLE" => Self::ManagerUnavailable,
            "E_PROTOCOL" => Self::Protocol,
            "E_CONFLICT" => Self::Conflict,
            "E_BUSY" => Self::Busy,
            "E_VERSION_MISMATCH" => Self::VersionMismatch,
            "E_NEEDS_CONFIRMATION" => Self::NeedsConfirmation,
            "E_NOT_FOUND" => Self::NotFound,
            "E_TIMEOUT" => Self::Timeout,
            "E_CANCELED" => Self::Canceled,
            "E_INTERRUPTED" => Self::Interrupted,
            "E_COMMAND" => Self::Command,
            "E_RELOAD" => Self::Reload,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BoxError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<String>,
    pub retryable: bool,
}

impl BoxError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            retryable: code.default_retryable(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Validation, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message)
    }

    pub fn manager_unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ManagerUnavailable, message)
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Permission, message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Protocol, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl fmt::Display for BoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for BoxError {}

impl From<io::Error> for BoxError {
    fn from(error: io::Error) -> Self {
        Self::new(ErrorCode::Io, error.to_string())
    }
}
