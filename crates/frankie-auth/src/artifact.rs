use crate::failure::AuthFailure;
use zeroize::Zeroizing;

/// What a successful D2L sign-in produces. The bearer is private: `Debug` hides it and there is no `Display`.
pub struct D2lArtifact {
    bearer: Zeroizing<String>,
    subject: String,
    pub elapsed_ms: u64,
}

impl D2lArtifact {
    pub(crate) fn new(bearer: String, subject: String, elapsed_ms: u64) -> Self {
        Self {
            bearer: Zeroizing::new(bearer),
            subject,
            elapsed_ms,
        }
    }
    /// The identity the bearer was verified against (D2L `whoami` identifier, not a secret but personal).
    pub fn subject(&self) -> &str {
        &self.subject
    }
    /// For the private connector only. Never log or return it from an API.
    pub fn bearer(&self) -> &str {
        &self.bearer
    }

    /// One JSON line in the shape `frankie-vault` requires (`deny_unknown_fields`, snake_case, includes
    /// `verified_subject`). Contains the bearer: write it only to the private stdout pipe of the caller.
    pub fn to_vault_json(&self) -> Zeroizing<String> {
        Zeroizing::new(
            serde_json::json!({
                "ok": true,
                "provider": "d2l",
                "credential_type": "bearer",
                "access_token": self.bearer.as_str(),
                "identity_verified": true,
                "verified_subject": self.subject,
                "http_status": 200,
                "elapsed_ms": self.elapsed_ms,
            })
            .to_string(),
        )
    }
}

impl std::fmt::Debug for D2lArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("D2lArtifact([REDACTED])")
    }
}

/// The failure line for the same pipe. Codes are the ones the vault parser understands; no secret is ever included.
pub fn failure_json(failure: AuthFailure) -> String {
    let error = match failure {
        AuthFailure::CredentialsRejected => "authentication_rejected",
        AuthFailure::ChallengeRequired(_) => "challenge_required",
        AuthFailure::Network | AuthFailure::Timeout => "upstream_unavailable",
        _ => "authentication_unverified",
    };
    serde_json::json!({ "ok": false, "provider": "d2l", "error": error }).to_string()
}
