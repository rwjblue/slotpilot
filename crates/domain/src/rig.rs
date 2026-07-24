//! Infrastructure-independent read-only rig profile values.

use std::{fmt, net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ProfileRevisionId;

const MAX_ENDPOINT_HOST_BYTES: usize = 253;
const MAX_HAMLIB_VERSION_BYTES: usize = 64;
const MIN_PASSBAND_HZ: u32 = 50;
const MAX_PASSBAND_HZ: u32 = 100_000;

/// Failure parsing or constructing a read-only rig profile value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RigProfileError {
    /// A network endpoint omitted either its explicit host or port.
    #[error("rig network endpoint must contain an explicit host and TCP port")]
    IncompleteEndpoint,
    /// The endpoint host is empty, oversized, or outside the portable alphabet.
    #[error("rig network endpoint host is invalid")]
    InvalidEndpointHost,
    /// The endpoint TCP port is zero or not an integer.
    #[error("rig network endpoint TCP port is invalid")]
    InvalidEndpointPort,
    /// A managed service endpoint could bind beyond the loopback interface.
    #[error("managed rigctld service endpoint must use a loopback IP literal")]
    ManagedServiceNotLoopback,
    /// The downstream radio and rigctld service endpoints were conflated.
    #[error("downstream rig CAT and rigctld service endpoints must be distinct")]
    EndpointRolesConflict,
    /// A Hamlib model identifier was zero.
    #[error("Hamlib model identifier must be nonzero")]
    InvalidHamlibModel,
    /// A Hamlib version expectation was empty, oversized, or nonportable.
    #[error("Hamlib version expectation is invalid")]
    InvalidHamlibVersion,
    /// A radio passband was outside the checked representation.
    #[error("radio passband must be between 50 and 100,000 Hz")]
    PassbandOutOfRange,
}

macro_rules! endpoint {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name {
            host: String,
            port: u16,
        }

        impl $name {
            /// Constructs an endpoint from an explicit host and TCP port.
            pub fn new(host: impl Into<String>, port: u16) -> Result<Self, RigProfileError> {
                let host = host.into();
                validate_host(&host)?;
                if port == 0 {
                    return Err(RigProfileError::InvalidEndpointPort);
                }
                Ok(Self { host, port })
            }

            /// Returns the exact configured host without resolving it.
            #[must_use]
            pub fn host(&self) -> &str {
                &self.host
            }

            /// Returns the explicit configured TCP port.
            #[must_use]
            pub const fn port(&self) -> u16 {
                self.port
            }
        }

        impl FromStr for $name {
            type Err = RigProfileError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let (host, port) = split_endpoint(value)?;
                Self::new(host, port)
            }
        }

        impl TryFrom<String> for $name {
            type Error = RigProfileError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.to_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}:{}", self.host, self.port)
            }
        }
    };
}

endpoint!(
    DownstreamRigEndpoint,
    "Exact operator-configured downstream radio CAT endpoint.\n\nThis type is structurally distinct from the local or external `rigctld`\nservice endpoint. It performs no discovery, resolution, or defaulting."
);
endpoint!(
    RigctldServiceEndpoint,
    "Exact endpoint at which SlotPilot contacts one `rigctld` service.\n\nThis type cannot be substituted for the downstream radio CAT endpoint."
);

/// How the future read-only adapter obtains its `rigctld` service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigctldMode {
    /// SlotPilot will manage a process bound to the configured loopback endpoint.
    ///
    /// The later lifecycle adapter must force PTT type `NONE`; this contract
    /// intentionally exposes no configurable PTT method.
    Managed,
    /// SlotPilot will connect to an explicitly configured existing service.
    External,
}

/// Nonzero Hamlib radio model identifier expected by a rig profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct HamlibModelId(u32);

impl HamlibModelId {
    /// Constructs a nonzero model identifier.
    pub fn new(value: u32) -> Result<Self, RigProfileError> {
        if value == 0 {
            return Err(RigProfileError::InvalidHamlibModel);
        }
        Ok(Self(value))
    }

    /// Returns the integer model identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for HamlibModelId {
    type Error = RigProfileError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<HamlibModelId> for u32 {
    fn from(value: HamlibModelId) -> Self {
        value.0
    }
}

/// Bounded exact Hamlib version expectation retained by a rig profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HamlibVersionExpectation(String);

impl HamlibVersionExpectation {
    /// Constructs a checked exact version expectation.
    pub fn new(value: impl Into<String>) -> Result<Self, RigProfileError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_HAMLIB_VERSION_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        {
            return Err(RigProfileError::InvalidHamlibVersion);
        }
        Ok(Self(value))
    }

    /// Returns the exact expected version text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HamlibVersionExpectation {
    type Error = RigProfileError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<HamlibVersionExpectation> for String {
    fn from(value: HamlibVersionExpectation) -> Self {
        value.0
    }
}

/// Radio-side modulation, structurally separate from synchronized protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadioModulation {
    /// Upper sideband.
    UpperSideband,
    /// Lower sideband.
    LowerSideband,
    /// Upper-sideband data mode.
    DataUpperSideband,
    /// Lower-sideband data mode.
    DataLowerSideband,
    /// Continuous wave.
    ContinuousWave,
    /// Reverse continuous wave.
    ContinuousWaveReverse,
    /// Amplitude modulation.
    AmplitudeModulation,
    /// Frequency modulation.
    FrequencyModulation,
    /// Radio teletype.
    RadioTeletype,
    /// Reverse radio teletype.
    RadioTeletypeReverse,
}

/// Exact radio passband width in integer hertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct RadioPassband(u32);

impl RadioPassband {
    /// Constructs a checked passband width.
    pub fn from_hz(value: u32) -> Result<Self, RigProfileError> {
        if !(MIN_PASSBAND_HZ..=MAX_PASSBAND_HZ).contains(&value) {
            return Err(RigProfileError::PassbandOutOfRange);
        }
        Ok(Self(value))
    }

    /// Returns the passband width in integer hertz.
    #[must_use]
    pub const fn hz(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for RadioPassband {
    type Error = RigProfileError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::from_hz(value)
    }
}

impl From<RadioPassband> for u32 {
    fn from(value: RadioPassband) -> Self {
        value.0
    }
}

/// Exact VFO identity reported by a read-only rig backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigVfo {
    /// VFO A.
    A,
    /// VFO B.
    B,
    /// Main receiver VFO.
    Main,
    /// Sub receiver VFO.
    Sub,
    /// Memory-channel context.
    Memory,
}

/// Exact split readback without inventing a transmit VFO when absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SplitReadback {
    enabled: bool,
    transmit_vfo: Option<RigVfo>,
}

impl SplitReadback {
    /// Constructs exact split and optional transmit-VFO readback.
    #[must_use]
    pub const fn new(enabled: bool, transmit_vfo: Option<RigVfo>) -> Self {
        Self {
            enabled,
            transmit_vfo,
        }
    }

    /// Returns the exact split flag.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Returns the transmit VFO only when the backend supplied one.
    #[must_use]
    pub const fn transmit_vfo(self) -> Option<RigVfo> {
        self.transmit_vfo
    }
}

/// Immutable, version-identified read-only rig profile contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RigProfileWire")]
pub struct RigProfile {
    revision_id: ProfileRevisionId,
    downstream_endpoint: DownstreamRigEndpoint,
    service_endpoint: RigctldServiceEndpoint,
    rigctld_mode: RigctldMode,
    hamlib_model: HamlibModelId,
    hamlib_version: HamlibVersionExpectation,
    required_modulation: RadioModulation,
    required_passband: RadioPassband,
}

#[derive(Deserialize)]
struct RigProfileWire {
    revision_id: ProfileRevisionId,
    downstream_endpoint: DownstreamRigEndpoint,
    service_endpoint: RigctldServiceEndpoint,
    rigctld_mode: RigctldMode,
    hamlib_model: HamlibModelId,
    hamlib_version: HamlibVersionExpectation,
    required_modulation: RadioModulation,
    required_passband: RadioPassband,
}

impl TryFrom<RigProfileWire> for RigProfile {
    type Error = RigProfileError;

    fn try_from(value: RigProfileWire) -> Result<Self, Self::Error> {
        Self::new(
            value.revision_id,
            value.downstream_endpoint,
            value.service_endpoint,
            value.rigctld_mode,
            value.hamlib_model,
            value.hamlib_version,
            value.required_modulation,
            value.required_passband,
        )
    }
}

impl RigProfile {
    /// Constructs a complete explicit read-only rig profile.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        revision_id: ProfileRevisionId,
        downstream_endpoint: DownstreamRigEndpoint,
        service_endpoint: RigctldServiceEndpoint,
        rigctld_mode: RigctldMode,
        hamlib_model: HamlibModelId,
        hamlib_version: HamlibVersionExpectation,
        required_modulation: RadioModulation,
        required_passband: RadioPassband,
    ) -> Result<Self, RigProfileError> {
        if downstream_endpoint.to_string() == service_endpoint.to_string() {
            return Err(RigProfileError::EndpointRolesConflict);
        }
        if rigctld_mode == RigctldMode::Managed && !is_loopback_literal(service_endpoint.host()) {
            return Err(RigProfileError::ManagedServiceNotLoopback);
        }
        Ok(Self {
            revision_id,
            downstream_endpoint,
            service_endpoint,
            rigctld_mode,
            hamlib_model,
            hamlib_version,
            required_modulation,
            required_passband,
        })
    }

    /// Returns the immutable profile revision identity.
    #[must_use]
    pub fn revision_id(&self) -> &ProfileRevisionId {
        &self.revision_id
    }

    /// Returns the exact downstream CAT endpoint.
    #[must_use]
    pub const fn downstream_endpoint(&self) -> &DownstreamRigEndpoint {
        &self.downstream_endpoint
    }

    /// Returns the distinct exact `rigctld` service endpoint.
    #[must_use]
    pub const fn service_endpoint(&self) -> &RigctldServiceEndpoint {
        &self.service_endpoint
    }

    /// Returns managed or external service ownership.
    #[must_use]
    pub const fn rigctld_mode(&self) -> RigctldMode {
        self.rigctld_mode
    }

    /// Returns the expected Hamlib model identifier.
    #[must_use]
    pub const fn hamlib_model(&self) -> HamlibModelId {
        self.hamlib_model
    }

    /// Returns the exact expected Hamlib version.
    #[must_use]
    pub const fn hamlib_version(&self) -> &HamlibVersionExpectation {
        &self.hamlib_version
    }

    /// Returns the required radio-side modulation.
    #[must_use]
    pub const fn required_modulation(&self) -> RadioModulation {
        self.required_modulation
    }

    /// Returns the required integer passband.
    #[must_use]
    pub const fn required_passband(&self) -> RadioPassband {
        self.required_passband
    }
}

fn split_endpoint(value: &str) -> Result<(&str, u16), RigProfileError> {
    let (host, port) = value
        .rsplit_once(':')
        .ok_or(RigProfileError::IncompleteEndpoint)?;
    let port = port
        .parse::<u16>()
        .map_err(|_| RigProfileError::InvalidEndpointPort)?;
    if port == 0 {
        return Err(RigProfileError::InvalidEndpointPort);
    }
    validate_host(host)?;
    Ok((host, port))
}

fn validate_host(host: &str) -> Result<(), RigProfileError> {
    if host.is_empty() || host.len() > MAX_ENDPOINT_HOST_BYTES {
        return Err(RigProfileError::InvalidEndpointHost);
    }
    if let Some(inner) = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        return inner
            .parse::<IpAddr>()
            .ok()
            .filter(IpAddr::is_ipv6)
            .map(|_| ())
            .ok_or(RigProfileError::InvalidEndpointHost);
    }
    if host.contains(':')
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(RigProfileError::InvalidEndpointHost);
    }
    Ok(())
}

fn is_loopback_literal(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    host.parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k4_profile() -> RigProfile {
        RigProfile::new(
            "prv_01jrigprofile".parse().unwrap(),
            DownstreamRigEndpoint::new("k4.operator.lan", 12_345).unwrap(),
            RigctldServiceEndpoint::new("127.0.0.1", 4_532).unwrap(),
            RigctldMode::Managed,
            HamlibModelId::new(2_047).unwrap(),
            HamlibVersionExpectation::new("4.7.1").unwrap(),
            RadioModulation::DataUpperSideband,
            RadioPassband::from_hz(3_000).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn profile_fixture_preserves_exact_distinct_endpoints_and_units() {
        let profile = k4_profile();
        let fixture = r#"{"revision_id":"prv_01jrigprofile","downstream_endpoint":"k4.operator.lan:12345","service_endpoint":"127.0.0.1:4532","rigctld_mode":"managed","hamlib_model":2047,"hamlib_version":"4.7.1","required_modulation":"data_upper_sideband","required_passband":3000}"#;
        assert_eq!(serde_json::to_string(&profile).unwrap(), fixture);
        assert_eq!(
            serde_json::from_str::<RigProfile>(fixture).unwrap(),
            profile
        );
        assert_eq!(profile.downstream_endpoint().port(), 12_345);
        assert_eq!(profile.service_endpoint().port(), 4_532);
    }

    #[test]
    fn endpoints_require_explicit_roles_hosts_and_ports() {
        assert_eq!(
            "k4.operator.lan"
                .parse::<DownstreamRigEndpoint>()
                .unwrap_err(),
            RigProfileError::IncompleteEndpoint
        );
        assert_eq!(
            "k4.operator.lan:0"
                .parse::<DownstreamRigEndpoint>()
                .unwrap_err(),
            RigProfileError::InvalidEndpointPort
        );
        assert_eq!(
            "unbracketed::1:4532"
                .parse::<RigctldServiceEndpoint>()
                .unwrap_err(),
            RigProfileError::InvalidEndpointHost
        );
        assert_eq!(
            "[::1]:4532"
                .parse::<RigctldServiceEndpoint>()
                .unwrap()
                .to_string(),
            "[::1]:4532"
        );
    }

    #[test]
    fn managed_profile_requires_loopback_and_distinct_endpoint() {
        let base = k4_profile();
        assert_eq!(
            RigProfile::new(
                base.revision_id().clone(),
                DownstreamRigEndpoint::new("k4.operator.lan", 12_345).unwrap(),
                RigctldServiceEndpoint::new("rigctld.operator.lan", 4_532).unwrap(),
                RigctldMode::Managed,
                base.hamlib_model(),
                base.hamlib_version().clone(),
                base.required_modulation(),
                base.required_passband(),
            )
            .unwrap_err(),
            RigProfileError::ManagedServiceNotLoopback
        );
        assert_eq!(
            RigProfile::new(
                base.revision_id().clone(),
                DownstreamRigEndpoint::new("127.0.0.1", 4_532).unwrap(),
                RigctldServiceEndpoint::new("127.0.0.1", 4_532).unwrap(),
                RigctldMode::External,
                base.hamlib_model(),
                base.hamlib_version().clone(),
                base.required_modulation(),
                base.required_passband(),
            )
            .unwrap_err(),
            RigProfileError::EndpointRolesConflict
        );
        let invalid_fixture = r#"{"revision_id":"prv_01jrigprofile","downstream_endpoint":"k4.operator.lan:12345","service_endpoint":"rigctld.operator.lan:4532","rigctld_mode":"managed","hamlib_model":2047,"hamlib_version":"4.7.1","required_modulation":"data_upper_sideband","required_passband":3000}"#;
        assert!(serde_json::from_str::<RigProfile>(invalid_fixture).is_err());
    }

    #[test]
    fn radio_modulation_is_not_a_protocol_operating_mode() {
        fn accepts_radio_mode(_: RadioModulation) {}
        accepts_radio_mode(RadioModulation::DataUpperSideband);
        assert_eq!(
            serde_json::to_string(&crate::OperatingMode::Ft8).unwrap(),
            r#""FT8""#
        );
        assert_eq!(
            serde_json::to_string(&RadioModulation::DataUpperSideband).unwrap(),
            r#""data_upper_sideband""#
        );
    }
}
