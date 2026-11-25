use crate::error::{ContractError, ToolError, ToolStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where the data came from. `Cached` lets an answer say "as of" when the device is offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Origin {
    Live,
    Simulated,
    Cached,
}

/// One dataset's completeness, so a partial answer is never presented as a full one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub dataset: String,
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The one result envelope. Wire names are snake_case; camelCase spellings emitted today are accepted on read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub contract_version: String,
    pub status: ToolStatus,
    pub origin: Origin,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fetchedAt")]
    pub fetched_at: Option<String>,
    pub complete: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "recordCount"
    )]
    pub record_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<Coverage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl ToolResult {
    pub fn pass(source: impl Into<String>, origin: Origin, data: Value) -> Self {
        let record_count = data.as_array().map(|a| a.len() as u64);
        Self {
            contract_version: crate::CONTRACT_VERSION.to_string(),
            status: ToolStatus::Pass,
            origin,
            source: source.into(),
            fetched_at: None,
            complete: true,
            record_count,
            data: Some(data),
            error: None,
            coverage: vec![],
            warnings: vec![],
        }
    }

    /// A failure: the status follows the error code (`NEEDS_LOGIN`, `NOT_CONFIGURED`, else `FAIL`).
    pub fn fail(source: impl Into<String>, origin: Origin, error: ToolError) -> Self {
        Self {
            contract_version: crate::CONTRACT_VERSION.to_string(),
            status: error.code.status(),
            origin,
            source: source.into(),
            fetched_at: None,
            complete: false,
            record_count: None,
            data: None,
            error: Some(error),
            coverage: vec![],
            warnings: vec![],
        }
    }

    /// The operation does not exist or is not supported on this source (producer's choice, not derived).
    pub fn unsupported(source: impl Into<String>, origin: Origin, error: ToolError) -> Self {
        let mut r = Self::fail(source, origin, error);
        r.status = ToolStatus::Unsupported;
        r
    }

    pub fn with_fetched_at(mut self, rfc3339: impl Into<String>) -> Self {
        self.fetched_at = Some(rfc3339.into());
        self
    }

    /// Structural rules the type system cannot express.
    pub fn validate(&self) -> Result<(), ContractError> {
        let bad = |m: &str| Err(ContractError::InvalidResult(m.to_string()));
        match self.status {
            ToolStatus::Pass => {
                if self.error.is_some() {
                    return bad("PASS must not carry an error");
                }
                if self.data.is_none() {
                    return bad("PASS must carry data");
                }
            }
            _ => {
                match &self.error {
                    None => return bad("a non-PASS result must carry an error"),
                    Some(e) if !e.is_consistent() => {
                        return bad("error.retryable disagrees with error.code")
                    }
                    Some(_) => {}
                }
                if self.data.is_some() {
                    return bad("a non-PASS result must not carry data");
                }
            }
        }
        if self.source.trim().is_empty() {
            return bad("source must not be empty");
        }
        if self.contract_version != crate::CONTRACT_VERSION {
            return bad("unsupported contract_version");
        }
        // Only the login-shaped codes may produce NEEDS_LOGIN/NOT_CONFIGURED.
        if let Some(e) = &self.error {
            let derived = e.code.status();
            let ok = self.status == derived
                || self.status == ToolStatus::Unsupported
                || self.status == ToolStatus::Blocked;
            if !ok {
                return bad("status does not follow error.code");
            }
        }
        Ok(())
    }
}
