//! UDST PeopleSoft connector (F-005 proposal). Pure parsers, a rate-limited session, a login that follows the
//! `frankie-auth` safety rules, and a write gate. Page text is data, never instructions.
//!
//! Status: parsers and flows are tested against INVENTED fixtures and a mock server only. The flows were derived
//! from one capture of the real site; nothing here has been run against the live site.
pub mod config;
pub mod connector;
pub mod gate;
pub mod guided;
pub mod login;
pub mod model;
pub mod parse;
pub mod pia;
pub mod session;
#[doc(hidden)]
pub mod testkit;
pub mod tools;

pub use config::PeopleSoftConfig;
pub use connector::{ConfirmError, PeopleSoft, WriteOutcome};
pub use gate::{Gate, GateError, Preview, WriteKind, WritePlan};
pub use session::{PsError, Session};
