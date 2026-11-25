use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("invalid descriptor: {0}")]
    InvalidDescriptor(String),
    #[error("invalid result: {0}")]
    InvalidResult(String),
    #[error("invalid json: {0}")]
    Json(String),
}

macro_rules! error_codes {
    ($($variant:ident => $text:literal),+ $(,)?) => {
        /// The one error vocabulary. Serialised as UPPER_SNAKE.
        ///
        /// The first 19 are the codes of `tool_error.schema.json`; the rest were added in F-001
        /// to cover codes already emitted by Navigate, Outlook, the vault and the prototype router.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum ErrorCode {
            $( #[serde(rename = $text)] $variant ),+
        }
        impl ErrorCode {
            pub const ALL: &'static [ErrorCode] = &[ $( ErrorCode::$variant ),+ ];
            pub const fn as_str(self) -> &'static str {
                match self { $( ErrorCode::$variant => $text ),+ }
            }
        }
    };
}

error_codes! {
    // --- the 19 codes of tool_error.schema.json ---
    InvalidInput => "INVALID_INPUT",
    InvalidCredentials => "INVALID_CREDENTIALS",
    AccountLocked => "ACCOUNT_LOCKED",
    AuthRejected => "AUTH_REJECTED",
    NetworkError => "NETWORK_ERROR",
    Timeout => "TIMEOUT",
    SessionExpired => "SESSION_EXPIRED",
    AccessDenied => "ACCESS_DENIED",
    RateLimited => "RATE_LIMITED",
    UnexpectedPage => "UNEXPECTED_PAGE",
    OriginRejected => "ORIGIN_REJECTED",
    RouteNotFound => "ROUTE_NOT_FOUND",
    CountMismatch => "COUNT_MISMATCH",
    NotImplemented => "NOT_IMPLEMENTED",
    NeedsLogin => "NEEDS_LOGIN",
    NotConfigured => "NOT_CONFIGURED",
    PermissionDenied => "PERMISSION_DENIED",
    NotFound => "NOT_FOUND",
    InvalidResponse => "INVALID_RESPONSE",
    // --- added in F-001 ---
    SourceChanged => "SOURCE_CHANGED",
    AccountMismatch => "ACCOUNT_MISMATCH",
    ChallengeRequired => "CHALLENGE_REQUIRED",
    ConfirmationRequired => "CONFIRMATION_REQUIRED",
    UnknownOrDisabledTool => "UNKNOWN_OR_DISABLED_TOOL",
    InternalError => "INTERNAL_ERROR",
    StorageError => "STORAGE_ERROR",
    CryptoError => "CRYPTO_ERROR",
    StaleData => "STALE_DATA",
    Conflict => "CONFLICT",
}

impl ErrorCode {
    /// The single source of truth for "is it worth retrying". Same four codes as the Java
    /// `ErrorCode.retryable` and the Rust `frankie-core` rule it replaces.
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::NetworkError
                | ErrorCode::Timeout
                | ErrorCode::SessionExpired
                | ErrorCode::RateLimited
        )
    }

    /// Status a failing operation reports. Only the login-shaped codes change the status; every
    /// other failure is plain `FAIL`. `UNSUPPORTED` and `BLOCKED` are chosen by the producer
    /// (for example when an operation does not exist), never derived from a code.
    pub const fn status(self) -> ToolStatus {
        match self {
            ErrorCode::NeedsLogin | ErrorCode::SessionExpired | ErrorCode::ChallengeRequired => {
                ToolStatus::NeedsLogin
            }
            ErrorCode::NotConfigured => ToolStatus::NotConfigured,
            _ => ToolStatus::Fail,
        }
    }
}

/// Outcome of one operation. `NOT_RUN` is a UI state, not a contract state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolStatus {
    Pass,
    Fail,
    Unsupported,
    NeedsLogin,
    NotConfigured,
    Blocked,
}

/// A typed, loggable error. `message` must be safe to log: never copy server text or secrets into it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolError {
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub message: String,
    pub retryable: bool,
    /// The producer's original code, kept so a mapping never loses information.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "sourceCode")]
    pub source_code: Option<String>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            stage: None,
            message: message.into(),
            retryable: code.retryable(),
            source_code: None,
        }
    }
    pub fn with_stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = Some(stage.into());
        self
    }
    pub fn with_source_code(mut self, source_code: impl Into<String>) -> Self {
        self.source_code = Some(source_code.into());
        self
    }
    /// Build from a code string emitted by an existing crate (any casing convention in use today).
    /// Returns `None` for a code the contract does not know, so a new producer code cannot slip by.
    pub fn from_legacy(source_code: &str, message: impl Into<String>) -> Option<Self> {
        let code = crate::legacy::from_legacy_code(source_code)?;
        Some(Self::new(code, message).with_source_code(source_code))
    }
    /// `retryable` must agree with the code; a hand-built or deserialised value may not.
    pub fn is_consistent(&self) -> bool {
        self.retryable == self.code.retryable()
    }
}
