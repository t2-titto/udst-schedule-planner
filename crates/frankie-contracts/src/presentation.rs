use crate::result::Origin;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Ok,
    NeedsLogin,
    NotConfigured,
    Failed,
    Unavailable,
    /// Data is shown but older than it should be; the screen must say "as of".
    Stale,
}

/// How one source (D2L, PeopleSoft, ...) is doing, shown next to the data it supplied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceStatus {
    pub source: String,
    pub state: SourceState,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CourseSummary {
    pub id: String,
    pub title: String,
    pub source: String,
}

/// Everything a native screen needs to draw "my courses": the data, where each part came from,
/// and any warning (partial coverage, stale source). Each platform renders this its own way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoursesScreen {
    pub courses: Vec<CourseSummary>,
    pub sources: Vec<SourceStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}
