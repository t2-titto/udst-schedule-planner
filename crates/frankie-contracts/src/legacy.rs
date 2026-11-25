use crate::error::ErrorCode::{self, *};

/// Every error-code string emitted today by the four existing crates, and the contract code it maps to.
/// Sources: frankie-core (`libcal_exec.rs`, `dataset_exec.rs`), navigate360-connector, frankie-outlook
/// (`Error::code`, `StoreError::code`), frankie-vault (`VaultError` Display, `AuthFailure::safe_code`).
/// Tests in each crate enumerate the real enum variants and fail if one is missing here.
pub const LEGACY_CODES: &[(&str, ErrorCode)] = &[
    // frankie-core (already contract codes)
    ("INVALID_INPUT", InvalidInput),
    ("INVALID_RESPONSE", InvalidResponse),
    ("NETWORK_ERROR", NetworkError),
    ("NOT_FOUND", NotFound),
    ("NOT_IMPLEMENTED", NotImplemented),
    ("ORIGIN_REJECTED", OriginRejected),
    ("PERMISSION_DENIED", PermissionDenied),
    ("RATE_LIMITED", RateLimited),
    ("TIMEOUT", Timeout),
    ("UNEXPECTED_PAGE", UnexpectedPage),
    ("NEEDS_LOGIN", NeedsLogin),
    ("NOT_CONFIGURED", NotConfigured),
    // navigate360-connector
    ("INVALID_REQUEST", InvalidInput),
    ("SOURCE_CHANGED", SourceChanged),
    ("INTERNAL", InternalError),
    ("NETWORK_UNAVAILABLE", NetworkError),
    ("INVALID_SESSION", NeedsLogin),
    ("AUTH_REQUIRED", NeedsLogin),
    ("ACCESS_DENIED", AccessDenied),
    ("ACCOUNT_MISMATCH", AccountMismatch),
    ("AUTH_PROVIDER_UNAVAILABLE", NotConfigured),
    ("AUTH_PROVIDER_ERROR", InvalidResponse),
    ("AUTH_PROVIDER_TIMEOUT", Timeout),
    ("DATA_TOO_LARGE", InvalidResponse),
    ("SOURCE_UNAVAILABLE", NetworkError),
    // frankie-outlook Error::code()
    ("invalid_message_id", InvalidInput),
    ("invalid_authorized_session", NeedsLogin),
    ("external_auth_unavailable", NotConfigured),
    ("outlook_source_authorization_rejected", AuthRejected),
    ("external_auth_invalid_response", InvalidResponse),
    ("outlook_transport_unavailable", NetworkError),
    ("outlook_throttled", RateLimited),
    ("outlook_http_error", NetworkError),
    ("outlook_protocol_error", InvalidResponse),
    ("outlook_item_not_returned", NotFound),
    ("outlook_response_invalid", InvalidResponse),
    ("encrypted_cache_unavailable", StorageError),
    ("stale_cache_requires_recent_sync_or_refresh", StaleData),
    ("index_reconciliation_required", StaleData),
    // frankie-outlook StoreError::code()
    ("storage_config_invalid", StorageError),
    ("storage_integrity_failed", StorageError),
    ("storage_io_failure", StorageError),
    ("storage_size_limit", StorageError),
    ("storage_account_mismatch", AccountMismatch),
    // frankie-vault VaultError (Display strings)
    ("invalid_input", InvalidInput),
    ("unsupported_service", NotImplemented),
    ("already_connected", Conflict),
    ("not_connected", NotConfigured),
    ("not_authorized", PermissionDenied),
    ("identity_mismatch", AccountMismatch),
    ("authentication_failed", AuthRejected),
    ("storage_unavailable", StorageError),
    ("cryptographic_failure", CryptoError),
    ("key_unavailable", CryptoError),
    // frankie-vault AuthFailure::safe_code()
    ("invalid_provider_response", InvalidResponse),
    ("credentials_rejected", InvalidCredentials),
    ("challenge_required", ChallengeRequired),
    ("upstream_unavailable", NetworkError),
    ("not_verified", AuthRejected),
    ("timeout", Timeout),
    // ("identity_mismatch" is already listed under VaultError)
    // prototype router only (Java), kept so the vocabulary is complete
    ("CONFIRMATION_REQUIRED", ConfirmationRequired),
    ("UNKNOWN_OR_DISABLED_TOOL", UnknownOrDisabledTool),
];

/// Map a code string emitted by an existing crate to the contract code. Exact, case-sensitive.
pub fn from_legacy_code(code: &str) -> Option<ErrorCode> {
    LEGACY_CODES
        .iter()
        .find(|(s, _)| *s == code)
        .map(|(_, c)| *c)
}
