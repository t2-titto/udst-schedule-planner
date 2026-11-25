//! Frankie shared contracts (F-001, v0.1.0).
//!
//! One vocabulary for errors, results, tool descriptors, operating modes and presentation state,
//! so connectors, the auth layer, storage and every UI describe the same things the same way.
//!
//! Rules: this crate does no I/O, holds no secrets, and depends on nothing in the repository.
//! Key order of JSON is never relied upon (another crate enables `serde_json/preserve_order`
//! for the whole workspace); anything hashed or signed must go through [`canonical_json`].

mod canonical;
mod descriptor;
mod error;
mod legacy;
mod mode;
mod presentation;
mod result;

pub use canonical::canonical_json;
pub use descriptor::{descriptors_from_catalog, SchemaRef, ToolDescriptor};
pub use error::{ContractError, ErrorCode, ToolError, ToolStatus};
pub use legacy::{from_legacy_code, LEGACY_CODES};
pub use mode::{policy, DataClass, Location, Mode, Policy, PolicyError, PolicyOptions};
pub use presentation::{CourseSummary, CoursesScreen, SourceState, SourceStatus};
pub use result::{Coverage, Origin, ToolResult};

/// Version of this contract. Every serialised [`ToolResult`] carries it.
pub const CONTRACT_VERSION: &str = "0.1.0";
