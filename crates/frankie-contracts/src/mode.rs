use serde::{Deserialize, Serialize};

/// The three operating modes. Not a global build flag: the design treats it as per-account scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Everything on the device; devices sync with each other end to end.
    Device,
    /// Credentials and sessions on the device; files and heavier data on our server.
    Hybrid,
    /// Everything on the server (credential custody is gated, see [`PolicyOptions`]).
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Credential,
    SessionArtifact,
    SourceEvidence,
    DerivedFact,
    File,
    Memory,
    Billing,
}

impl DataClass {
    pub const ALL: &'static [DataClass] = &[
        DataClass::Credential,
        DataClass::SessionArtifact,
        DataClass::SourceEvidence,
        DataClass::DerivedFact,
        DataClass::File,
        DataClass::Memory,
        DataClass::Billing,
    ];
    /// Passwords, tokens and cookies: the classes that must never leave the device outside Server mode.
    pub const fn is_secret(self) -> bool {
        matches!(self, DataClass::Credential | DataClass::SessionArtifact)
    }
}

impl Mode {
    pub const ALL: &'static [Mode] = &[Mode::Device, Mode::Hybrid, Mode::Server];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Device,
    Server,
    DeviceAndServerSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub location: Location,
    /// May this data class be copied to another of the user's devices or to the server?
    pub may_sync: bool,
    pub requires_consent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Server custody of university credentials is off until the permission question (GATE-003 / D4) is answered.
    #[error("server custody of secrets is disabled")]
    ServerCustodyDisabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PolicyOptions {
    /// Must stay `false` until UDST's terms on third-party credential custody have been resolved.
    pub server_credential_custody: bool,
}

/// What may live where. Encodes T2's definitions of 2026-11-11; the Hybrid file placement and the
/// Device-mode sync scope still differ from the earlier written designs (open decisions D2, D3).
pub fn policy(mode: Mode, class: DataClass, opts: PolicyOptions) -> Result<Policy, PolicyError> {
    use DataClass::*;
    use Location::*;
    let p = |location, may_sync, requires_consent| {
        Ok(Policy {
            location,
            may_sync,
            requires_consent,
        })
    };
    match mode {
        // Everything stays on the device. Secrets never sync (D2); everything else may sync device-to-device.
        Mode::Device => match class {
            Credential | SessionArtifact => p(Device, false, false),
            _ => p(Device, true, false),
        },
        // Secrets stay on the device. Files and billing live on the server; facts and memory sync with consent.
        Mode::Hybrid => match class {
            Credential | SessionArtifact => p(Device, false, false),
            SourceEvidence => p(Device, false, false),
            DerivedFact | Memory => p(DeviceAndServerSync, true, true),
            File => p(Server, true, true),
            Billing => p(Server, true, false),
        },
        // Everything on the server, but secrets only if custody has been explicitly enabled.
        Mode::Server => match class {
            Credential | SessionArtifact if !opts.server_credential_custody => {
                Err(PolicyError::ServerCustodyDisabled)
            }
            Credential | SessionArtifact => p(Server, false, true),
            _ => p(Server, true, true),
        },
    }
}
