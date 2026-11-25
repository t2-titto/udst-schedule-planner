use crate::challenge::Challenge;
use frankie_contracts::{ErrorCode, ToolError};

/// Why a login stopped. Messages are fixed text and never contain credentials, tokens or page content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthFailure {
    #[error("sign-in challenge: {0:?}")]
    ChallengeRequired(Challenge),
    #[error("credentials rejected")]
    CredentialsRejected,
    #[error("unexpected page")]
    UnexpectedPage,
    #[error("host not allowed")]
    HostNotAllowed,
    #[error("redirect not followed")]
    RedirectRefused,
    #[error("redirect limit reached")]
    RedirectLimit,
    #[error("SAML target is not the exact ACS")]
    AcsMismatch,
    #[error("sign-in configuration incomplete")]
    ConfigIncomplete,
    #[error("network error")]
    Network,
    #[error("timeout")]
    Timeout,
    #[error("identity mismatch")]
    IdentityMismatch,
    #[error("invalid input")]
    InvalidInput,
}

impl AuthFailure {
    /// Short stable code, the same spirit as the PowerShell `Halt` reasons.
    pub fn safe_code(self) -> &'static str {
        match self {
            Self::ChallengeRequired(_) => "challenge_required",
            Self::CredentialsRejected => "credentials_rejected",
            Self::UnexpectedPage => "unexpected_page",
            Self::HostNotAllowed => "unexpected_https_host_or_scheme",
            Self::RedirectRefused => "non_get_redirect_not_followed_to_avoid_credential_replay",
            Self::RedirectLimit => "redirect_limit_reached",
            Self::AcsMismatch => "acs_mismatch",
            Self::ConfigIncomplete => "microsoft_signin_config_incomplete",
            Self::Network => "network_error",
            Self::Timeout => "timeout",
            Self::IdentityMismatch => "identity_mismatch",
            Self::InvalidInput => "invalid_input",
        }
    }

    /// Whether the caller may run the login again automatically. Never after a rejection or a challenge:
    /// repeating either risks locking the account.
    pub fn may_retry_automatically(self) -> bool {
        matches!(self, Self::Network | Self::Timeout)
    }

    pub fn contract_code(self) -> ErrorCode {
        match self {
            Self::ChallengeRequired(_) => ErrorCode::ChallengeRequired,
            Self::CredentialsRejected => ErrorCode::InvalidCredentials,
            Self::UnexpectedPage | Self::ConfigIncomplete => ErrorCode::UnexpectedPage,
            Self::HostNotAllowed | Self::RedirectRefused | Self::AcsMismatch => {
                ErrorCode::OriginRejected
            }
            Self::RedirectLimit => ErrorCode::UnexpectedPage,
            Self::Network => ErrorCode::NetworkError,
            Self::Timeout => ErrorCode::Timeout,
            Self::IdentityMismatch => ErrorCode::AccountMismatch,
            Self::InvalidInput => ErrorCode::InvalidInput,
        }
    }

    pub fn to_contract(self) -> ToolError {
        ToolError::new(self.contract_code(), self.to_string())
            .with_stage("sign_in")
            .with_source_code(self.safe_code())
    }
}
