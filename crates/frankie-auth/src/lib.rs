//! Frankie login flows, ported from ABED's PowerShell (F-003).
//!
//! Parsers and safety rules are pure. The only code that touches the network is `transport.rs` (feature
//! `http-client`); there is no process spawning, file access or `unsafe` anywhere, and a test enforces that.
//! Source of truth for behavior: `legacy/auth-powershell/portable/d2l_auth.ps1`.
//!
//! Safety rules encoded here (each has tests):
//! * redirects: https only, default port, no userinfo, allow-listed host, GET only, never replay a POST;
//! * the SAML form must be a POST to the *exact* ACS (scheme, host, port, path; no query, fragment or userinfo)
//!   and carry a non-empty `SAMLResponse`;
//! * a sign-in challenge (MFA, policy, device, consent) stops the flow; nothing here tries to complete one;
//! * credentials live in zeroizing memory and never appear in `Debug` or error text.

mod artifact;
mod challenge;
mod config;
mod credentials;
mod d2l;
mod failure;
mod html;
mod http;
mod navigate;
mod redirect;
#[cfg(feature = "http-client")]
mod transport;

pub use artifact::{failure_json, D2lArtifact};
pub use challenge::{classify_challenge, Challenge};
pub use config::{extract_signin_config, page_config_json, SigninConfig};
pub use credentials::{Credentials, CredentialsError};
pub use d2l::{authenticate_d2l, D2lConfig};
pub use failure::AuthFailure;
pub use html::{exactly_one, find_saml_forms, get_attribute, AcsRule, SamlForm};
pub use http::{CookieMode, Http, HttpRequest, HttpResponse, Method};
pub use navigate::{authenticate_navigate, NavigateArtifact, NavigateConfig};
pub use redirect::{follow_redirect, is_allowed_uri, RedirectDecision, MAX_REDIRECTS};
#[cfg(feature = "http-client")]
pub use transport::{ReqwestHttp, MAX_BODY_BYTES};
