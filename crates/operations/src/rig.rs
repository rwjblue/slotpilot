//! Consumer-owned contracts for read-only Phase 3 rig observation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use slotpilot_domain::{
    DialFrequency, Power, ProfileRevisionId, RadioModulation, RadioPassband, RigProfile, RigVfo,
    ServiceInstanceId, SplitReadback,
};
use thiserror::Error;

use crate::ClockSample;

/// Number of distinct read capabilities in the Phase 3 contract.
pub const MAX_RIG_CAPABILITIES: usize = 7;
/// Maximum findings retained in one bounded profile validation.
pub const MAX_RIG_VALIDATION_FINDINGS: usize = 32;
const MAX_RIG_FRESHNESS_MILLIS: u64 = 300_000;
const MAX_RIG_TIMING_DIVERGENCE_MILLIS: u64 = 60_000;

/// Failure constructing a bounded read-only rig contract value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RigContractError {
    /// A connection generation or observation sequence was zero.
    #[error("rig generation and sequence values must be nonzero")]
    ZeroGeneration,
    /// Capability evidence was duplicated or exceeded the bounded set.
    #[error("rig capability evidence must be unique and bounded")]
    InvalidCapabilitySet,
    /// Profile validation findings exceeded their fixed bound.
    #[error("rig profile validation findings exceed the fixed bound")]
    TooManyValidationFindings,
    /// A UTC timestamp preceded the Unix epoch.
    #[error("rig observation UTC timestamp precedes the Unix epoch")]
    NegativeUtcTimestamp,
    /// A freshness policy was zero or exceeded its fixed bound.
    #[error("rig freshness policy is outside the fixed bound")]
    InvalidFreshnessPolicy,
}

/// One read operation whose support can be evaluated independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigCapability {
    /// Read the dial frequency.
    DialFrequency,
    /// Read the radio modulation.
    Modulation,
    /// Read the radio passband.
    Passband,
    /// Read the selected VFO.
    Vfo,
    /// Read split state and optional transmit VFO.
    Split,
    /// Read configured power.
    Power,
    /// Read PTT state as evidence only.
    Ptt,
}

/// Strength or failure state of evidence for one read capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigCapabilityStatus {
    /// The backend advertised support, but no getter has succeeded.
    BackendClaimed,
    /// A bounded runtime getter probe succeeded.
    RuntimeProbed,
    /// A human physical-validation record confirmed the getter.
    PhysicallyVerified,
    /// The backend explicitly does not support the getter.
    Unsupported,
    /// The getter could not be evaluated during this connection.
    Unavailable,
    /// Evidence is absent, expired, or otherwise not verified.
    StaleOrUnverified,
}

/// Evidence for exactly one read capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigCapabilityEvidence {
    /// Read operation being described.
    pub capability: RigCapability,
    /// Current strength or failure state.
    pub status: RigCapabilityStatus,
}

/// Positive generation of one persistent read-only rig connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct RigConnectionGeneration(u64);

impl RigConnectionGeneration {
    /// Constructs a nonzero generation.
    pub fn new(value: u64) -> Result<Self, RigContractError> {
        if value == 0 {
            return Err(RigContractError::ZeroGeneration);
        }
        Ok(Self(value))
    }

    /// Returns the integer generation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for RigConnectionGeneration {
    type Error = RigContractError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RigConnectionGeneration> for u64 {
    fn from(value: RigConnectionGeneration) -> Self {
        value.0
    }
}

/// Positive sequence of one observation within a connection generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct RigObservationSequence(u64);

impl RigObservationSequence {
    /// Constructs a nonzero sequence.
    pub fn new(value: u64) -> Result<Self, RigContractError> {
        if value == 0 {
            return Err(RigContractError::ZeroGeneration);
        }
        Ok(Self(value))
    }

    /// Returns the integer sequence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for RigObservationSequence {
    type Error = RigContractError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RigObservationSequence> for u64 {
    fn from(value: RigObservationSequence) -> Self {
        value.0
    }
}

/// Bounded capability report tied to one connection generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RigCapabilityReportWire")]
pub struct RigCapabilityReport {
    generation: RigConnectionGeneration,
    evidence: Vec<RigCapabilityEvidence>,
}

#[derive(Deserialize)]
struct RigCapabilityReportWire {
    generation: RigConnectionGeneration,
    evidence: Vec<RigCapabilityEvidence>,
}

impl TryFrom<RigCapabilityReportWire> for RigCapabilityReport {
    type Error = RigContractError;

    fn try_from(value: RigCapabilityReportWire) -> Result<Self, Self::Error> {
        Self::new(value.generation, value.evidence)
    }
}

impl RigCapabilityReport {
    /// Constructs a report after enforcing fixed size and unique capabilities.
    pub fn new(
        generation: RigConnectionGeneration,
        evidence: Vec<RigCapabilityEvidence>,
    ) -> Result<Self, RigContractError> {
        if evidence.len() > MAX_RIG_CAPABILITIES {
            return Err(RigContractError::InvalidCapabilitySet);
        }
        let unique: BTreeSet<_> = evidence.iter().map(|item| item.capability).collect();
        if unique.len() != evidence.len() {
            return Err(RigContractError::InvalidCapabilitySet);
        }
        Ok(Self {
            generation,
            evidence,
        })
    }

    /// Returns the connection generation that produced this report.
    #[must_use]
    pub const fn generation(&self) -> RigConnectionGeneration {
        self.generation
    }

    /// Returns bounded evidence in adapter-defined probe order.
    #[must_use]
    pub fn evidence(&self) -> &[RigCapabilityEvidence] {
        &self.evidence
    }

    /// Returns evidence for one capability, if the probe reported it.
    #[must_use]
    pub fn status(&self, capability: RigCapability) -> Option<RigCapabilityStatus> {
        self.evidence
            .iter()
            .find(|item| item.capability == capability)
            .map(|item| item.status)
    }
}

/// Readback state that never substitutes a primitive default for missing data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum RigReadback<T> {
    /// A getter returned an exact owned value.
    Observed(T),
    /// The getter is explicitly unsupported.
    Unsupported,
    /// The getter was temporarily unavailable.
    Unavailable,
    /// The value is absent, stale, or otherwise unverified.
    StaleOrUnverified,
}

/// Exact owned fields from one read-only observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigObservationFields {
    /// Dial-frequency readback in integer hertz.
    pub dial_frequency: RigReadback<DialFrequency>,
    /// Radio modulation, never a synchronized protocol mode.
    pub modulation: RigReadback<RadioModulation>,
    /// Passband readback in integer hertz.
    pub passband: RigReadback<RadioPassband>,
    /// Exact selected VFO.
    pub vfo: RigReadback<RigVfo>,
    /// Exact split and optional transmit-VFO readback.
    pub split: RigReadback<SplitReadback>,
    /// Optional power getter; absence cannot become zero.
    pub power: RigReadback<Power>,
    /// Optional PTT getter as evidence only; absence cannot become false.
    pub ptt_asserted: RigReadback<bool>,
}

/// Paired UTC and process-local monotonic timestamp for a rig observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RigObservationTimestampWire")]
pub struct RigObservationTimestamp {
    utc_unix_millis: i64,
    monotonic_millis: u64,
}

#[derive(Deserialize)]
struct RigObservationTimestampWire {
    utc_unix_millis: i64,
    monotonic_millis: u64,
}

impl TryFrom<RigObservationTimestampWire> for RigObservationTimestamp {
    type Error = RigContractError;

    fn try_from(value: RigObservationTimestampWire) -> Result<Self, Self::Error> {
        Self::new(value.utc_unix_millis, value.monotonic_millis)
    }
}

impl RigObservationTimestamp {
    /// Constructs a checked paired timestamp.
    pub fn new(utc_unix_millis: i64, monotonic_millis: u64) -> Result<Self, RigContractError> {
        if utc_unix_millis < 0 {
            return Err(RigContractError::NegativeUtcTimestamp);
        }
        Ok(Self {
            utc_unix_millis,
            monotonic_millis,
        })
    }

    /// Captures the owned integer values from an injected clock sample.
    #[must_use]
    pub fn from_clock_sample(sample: ClockSample) -> Self {
        Self {
            utc_unix_millis: sample.utc.unix_millis(),
            monotonic_millis: sample.monotonic.millis(),
        }
    }

    /// Returns milliseconds since the Unix epoch.
    #[must_use]
    pub const fn utc_unix_millis(self) -> i64 {
        self.utc_unix_millis
    }

    /// Returns milliseconds from the daemon process monotonic origin.
    #[must_use]
    pub const fn monotonic_millis(self) -> u64 {
        self.monotonic_millis
    }
}

/// Source and ordering identity retained with every observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigObservationProvenance {
    /// Daemon process identity whose monotonic origin applies.
    pub service_instance_id: ServiceInstanceId,
    /// Persistent connection generation.
    pub connection_generation: RigConnectionGeneration,
    /// Sequence within that connection.
    pub sequence: RigObservationSequence,
}

/// One complete owned read-only observation before freshness admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigObservation {
    /// Immutable profile revision used for the read.
    pub profile_revision_id: ProfileRevisionId,
    /// Source process, connection, and sequence.
    pub provenance: RigObservationProvenance,
    /// Paired observation time.
    pub observed_at: RigObservationTimestamp,
    /// Exact values and explicit missing states.
    pub fields: RigObservationFields,
}

impl RigObservation {
    /// Checks paired age, timing consistency, and monotonic freshness.
    pub fn fresh_at(
        &self,
        now: ClockSample,
        policy: RigFreshnessPolicy,
    ) -> Result<RigObservationAge, RigFault> {
        let now_utc = now.utc.unix_millis();
        let now_monotonic = now.monotonic.millis();
        let utc_delta = now_utc
            .checked_sub(self.observed_at.utc_unix_millis)
            .ok_or(RigFault::TimelineRegressed)?;
        let monotonic_delta = now_monotonic
            .checked_sub(self.observed_at.monotonic_millis)
            .ok_or(RigFault::TimelineRegressed)?;
        let utc_millis = u64::try_from(utc_delta).map_err(|_| RigFault::TimelineRegressed)?;
        let divergence = utc_millis.abs_diff(monotonic_delta);
        if divergence > policy.max_timing_divergence_millis {
            return Err(RigFault::ContradictoryTiming {
                utc_age_millis: utc_millis,
                monotonic_age_millis: monotonic_delta,
            });
        }
        if monotonic_delta > policy.max_age_millis {
            return Err(RigFault::Stale {
                age_millis: monotonic_delta,
                maximum_millis: policy.max_age_millis,
            });
        }
        Ok(RigObservationAge {
            utc_millis,
            monotonic_millis: monotonic_delta,
        })
    }
}

/// Checked paired age of a fresh rig observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigObservationAge {
    utc_millis: u64,
    monotonic_millis: u64,
}

impl RigObservationAge {
    /// Returns UTC-derived age.
    #[must_use]
    pub const fn utc_millis(self) -> u64 {
        self.utc_millis
    }

    /// Returns process-monotonic age used for freshness admission.
    #[must_use]
    pub const fn monotonic_millis(self) -> u64 {
        self.monotonic_millis
    }
}

/// Fixed limits for freshness and UTC/monotonic consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RigFreshnessPolicy {
    max_age_millis: u64,
    max_timing_divergence_millis: u64,
}

impl RigFreshnessPolicy {
    /// Constructs a checked freshness policy.
    pub fn new(
        max_age_millis: u64,
        max_timing_divergence_millis: u64,
    ) -> Result<Self, RigContractError> {
        if max_age_millis == 0
            || max_age_millis > MAX_RIG_FRESHNESS_MILLIS
            || max_timing_divergence_millis > MAX_RIG_TIMING_DIVERGENCE_MILLIS
        {
            return Err(RigContractError::InvalidFreshnessPolicy);
        }
        Ok(Self {
            max_age_millis,
            max_timing_divergence_millis,
        })
    }

    /// Returns the maximum admitted monotonic age.
    #[must_use]
    pub const fn max_age_millis(self) -> u64 {
        self.max_age_millis
    }

    /// Returns the maximum UTC/monotonic age divergence.
    #[must_use]
    pub const fn max_timing_divergence_millis(self) -> u64 {
        self.max_timing_divergence_millis
    }
}

/// Profile-validation outcome for one required read operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigValidationFinding {
    /// Required read operation.
    pub capability: RigCapability,
    /// Available evidence or explicit failure state.
    pub status: RigCapabilityStatus,
    /// Whether this evidence is sufficient for read-only verification.
    pub disposition: RigValidationDisposition,
}

/// Effect of one validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigValidationDisposition {
    /// Runtime or physical evidence satisfies this read requirement.
    Satisfied,
    /// Claimed-only or stale evidence requires a visible follow-up.
    Unverified,
    /// The required getter is unsupported or unavailable.
    Failed,
}

/// Bounded validation result tied to one immutable profile revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RigProfileValidationWire")]
pub struct RigProfileValidation {
    profile_revision_id: ProfileRevisionId,
    findings: Vec<RigValidationFinding>,
}

#[derive(Deserialize)]
struct RigProfileValidationWire {
    profile_revision_id: ProfileRevisionId,
    findings: Vec<RigValidationFinding>,
}

impl TryFrom<RigProfileValidationWire> for RigProfileValidation {
    type Error = RigContractError;

    fn try_from(value: RigProfileValidationWire) -> Result<Self, Self::Error> {
        Self::new(value.profile_revision_id, value.findings)
    }
}

impl RigProfileValidation {
    /// Constructs a bounded validation result.
    pub fn new(
        profile_revision_id: ProfileRevisionId,
        findings: Vec<RigValidationFinding>,
    ) -> Result<Self, RigContractError> {
        if findings.len() > MAX_RIG_VALIDATION_FINDINGS {
            return Err(RigContractError::TooManyValidationFindings);
        }
        Ok(Self {
            profile_revision_id,
            findings,
        })
    }

    /// Returns the immutable profile revision evaluated.
    #[must_use]
    pub fn profile_revision_id(&self) -> &ProfileRevisionId {
        &self.profile_revision_id
    }

    /// Returns bounded findings in requirement order.
    #[must_use]
    pub fn findings(&self) -> &[RigValidationFinding] {
        &self.findings
    }

    /// Returns whether every required capability has sufficient evidence.
    #[must_use]
    pub fn is_satisfied(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| finding.disposition == RigValidationDisposition::Satisfied)
    }
}

/// Validates required reads without treating backend claims as probe evidence.
pub fn validate_profile_capabilities(
    profile: &RigProfile,
    required: &[RigCapability],
    report: &RigCapabilityReport,
) -> Result<RigProfileValidation, RigContractError> {
    if required.len() > MAX_RIG_CAPABILITIES {
        return Err(RigContractError::InvalidCapabilitySet);
    }
    let unique: BTreeSet<_> = required.iter().copied().collect();
    if unique.len() != required.len() {
        return Err(RigContractError::InvalidCapabilitySet);
    }
    let findings = required
        .iter()
        .map(|capability| {
            let status = report
                .status(*capability)
                .unwrap_or(RigCapabilityStatus::StaleOrUnverified);
            let disposition = match status {
                RigCapabilityStatus::RuntimeProbed | RigCapabilityStatus::PhysicallyVerified => {
                    RigValidationDisposition::Satisfied
                }
                RigCapabilityStatus::BackendClaimed | RigCapabilityStatus::StaleOrUnverified => {
                    RigValidationDisposition::Unverified
                }
                RigCapabilityStatus::Unsupported | RigCapabilityStatus::Unavailable => {
                    RigValidationDisposition::Failed
                }
            };
            RigValidationFinding {
                capability: *capability,
                status,
                disposition,
            }
        })
        .collect();
    RigProfileValidation::new(profile.revision_id().clone(), findings)
}

/// Read-only operation associated with a typed failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigOperation {
    /// Establish or re-establish a connection.
    Connect,
    /// Probe read capabilities.
    Probe,
    /// Read one observation.
    Read,
}

/// Readback field associated with contradictory or unexpected evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigObservedField {
    /// Immutable rig profile revision.
    ProfileRevision,
    /// Dial frequency.
    DialFrequency,
    /// Radio modulation.
    Modulation,
    /// Passband.
    Passband,
    /// VFO.
    Vfo,
    /// Split state.
    Split,
    /// Power.
    Power,
    /// PTT evidence.
    Ptt,
    /// Paired UTC/monotonic time.
    Timing,
}

/// Stable coarse kind used by lifecycle snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigFaultKind {
    /// No connection has completed.
    NotConnected,
    /// Capability probing has not completed.
    NotProbed,
    /// The connection disappeared.
    Disconnected,
    /// An operation timed out.
    Timeout,
    /// Readback is too old.
    Stale,
    /// UTC or monotonic time moved backwards.
    TimelineRegressed,
    /// UTC and monotonic age evidence contradicted.
    Contradictory,
    /// A required getter is unsupported.
    Unsupported,
    /// A response was malformed or the deterministic sequence was violated.
    Malformed,
    /// State changed outside the expected read-only profile.
    UnexpectedChange,
    /// Evidence belongs to a different connection generation.
    GenerationChanged,
}

/// Typed read-only rig failure.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum RigFault {
    /// A probe or read was attempted before connection.
    #[error("rig is not connected")]
    NotConnected,
    /// A read was attempted before a successful capability probe.
    #[error("rig capabilities have not been probed")]
    NotProbed,
    /// The rig connection is unavailable.
    #[error("rig disconnected")]
    Disconnected,
    /// A bounded operation exceeded its deadline.
    #[error("rig {operation:?} timed out")]
    Timeout {
        /// Operation that timed out.
        operation: RigOperation,
    },
    /// Readback exceeded the caller's freshness requirement.
    #[error("rig readback age {age_millis} ms exceeds {maximum_millis} ms")]
    Stale {
        /// Observed monotonic age.
        age_millis: u64,
        /// Maximum admitted age.
        maximum_millis: u64,
    },
    /// UTC or monotonic time moved backwards.
    #[error("rig observation timeline regressed")]
    TimelineRegressed,
    /// UTC-derived and monotonic age evidence diverged.
    #[error("rig observation UTC and monotonic age contradict")]
    ContradictoryTiming {
        /// UTC-derived age.
        utc_age_millis: u64,
        /// Monotonic-derived age.
        monotonic_age_millis: u64,
    },
    /// Readback fields contradict one another or the expected profile.
    #[error("rig readback is contradictory for {field:?}")]
    ContradictoryReadback {
        /// Contradictory field.
        field: RigObservedField,
    },
    /// The adapter reports that a required read is unsupported.
    #[error("rig read capability {capability:?} is unsupported")]
    Unsupported {
        /// Unsupported getter.
        capability: RigCapability,
    },
    /// A bounded response was malformed.
    #[error("rig response is malformed")]
    MalformedResponse,
    /// A read-only observation changed outside the expected profile.
    #[error("rig changed unexpectedly at {field:?}")]
    UnexpectedChange {
        /// Changed field.
        field: RigObservedField,
    },
    /// Evidence belongs to a different connection generation.
    #[error("rig connection generation changed from {expected:?} to {observed:?}")]
    GenerationChanged {
        /// Generation expected by the consumer.
        expected: RigConnectionGeneration,
        /// Generation attached to the evidence.
        observed: RigConnectionGeneration,
    },
}

impl RigFault {
    /// Returns the stable lifecycle fault kind.
    #[must_use]
    pub const fn kind(&self) -> RigFaultKind {
        match self {
            Self::NotConnected => RigFaultKind::NotConnected,
            Self::NotProbed => RigFaultKind::NotProbed,
            Self::Disconnected => RigFaultKind::Disconnected,
            Self::Timeout { .. } => RigFaultKind::Timeout,
            Self::Stale { .. } => RigFaultKind::Stale,
            Self::TimelineRegressed => RigFaultKind::TimelineRegressed,
            Self::ContradictoryTiming { .. } | Self::ContradictoryReadback { .. } => {
                RigFaultKind::Contradictory
            }
            Self::Unsupported { .. } => RigFaultKind::Unsupported,
            Self::MalformedResponse => RigFaultKind::Malformed,
            Self::UnexpectedChange { .. } => RigFaultKind::UnexpectedChange,
            Self::GenerationChanged { .. } => RigFaultKind::GenerationChanged,
        }
    }
}

/// Observable lifecycle of the read-only rig port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RigLifecycleState {
    /// No connection exists.
    Disconnected,
    /// Connection establishment is in progress.
    Connecting,
    /// Connected but not yet probed.
    Connected {
        /// Current generation.
        generation: RigConnectionGeneration,
    },
    /// Capability probing is in progress.
    Probing {
        /// Current generation.
        generation: RigConnectionGeneration,
    },
    /// The connection has current probe evidence and can be read.
    Ready {
        /// Current generation.
        generation: RigConnectionGeneration,
    },
    /// The port failed visibly and retains no verified fresh state.
    Faulted {
        /// Last known generation, when one existed.
        generation: Option<RigConnectionGeneration>,
        /// Stable typed failure kind.
        fault: RigFaultKind,
    },
}

/// Consumer-owned production-facing Phase 3 rig boundary.
///
/// This trait deliberately exposes only connection, capability probing,
/// observation, and lifecycle inspection. It has no setter, raw command, PTT,
/// unkey, output-audio, scheduling, or transmit-authority operation.
pub trait ReadOnlyRigPort {
    /// Returns the current visible lifecycle state.
    fn lifecycle(&self) -> RigLifecycleState;
    /// Establishes or re-establishes one profile-bound connection.
    fn connect(&mut self, profile: &RigProfile) -> Result<RigConnectionGeneration, RigFault>;
    /// Probes actual read operations for the current connection.
    fn probe(&mut self) -> Result<RigCapabilityReport, RigFault>;
    /// Reads one owned observation from a successfully probed connection.
    fn read(&mut self) -> Result<RigObservation, RigFault>;
}

#[cfg(test)]
mod tests {
    use slotpilot_domain::{
        DownstreamRigEndpoint, HamlibModelId, HamlibVersionExpectation, RadioModulation,
        RadioPassband, RigctldMode, RigctldServiceEndpoint,
    };

    use super::*;
    use crate::{MonotonicInstant, UtcInstant};

    fn profile() -> RigProfile {
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
    fn capability_validation_preserves_every_evidence_level() {
        let statuses = [
            RigCapabilityStatus::BackendClaimed,
            RigCapabilityStatus::RuntimeProbed,
            RigCapabilityStatus::PhysicallyVerified,
            RigCapabilityStatus::Unsupported,
            RigCapabilityStatus::Unavailable,
            RigCapabilityStatus::StaleOrUnverified,
        ];
        let capabilities = [
            RigCapability::DialFrequency,
            RigCapability::Modulation,
            RigCapability::Passband,
            RigCapability::Vfo,
            RigCapability::Split,
            RigCapability::Power,
        ];
        let report = RigCapabilityReport::new(
            RigConnectionGeneration::new(1).unwrap(),
            capabilities
                .iter()
                .zip(statuses)
                .map(|(capability, status)| RigCapabilityEvidence {
                    capability: *capability,
                    status,
                })
                .collect(),
        )
        .unwrap();
        let validation = validate_profile_capabilities(&profile(), &capabilities, &report).unwrap();
        assert_eq!(
            validation
                .findings()
                .iter()
                .map(|finding| finding.status)
                .collect::<Vec<_>>(),
            statuses
        );
        assert!(!validation.is_satisfied());
    }

    #[test]
    fn observation_fixture_preserves_absent_optional_values_and_provenance() {
        let observation = RigObservation {
            profile_revision_id: "prv_01jrigprofile".parse().unwrap(),
            provenance: RigObservationProvenance {
                service_instance_id: "svc_01jrigprocess".parse().unwrap(),
                connection_generation: RigConnectionGeneration::new(2).unwrap(),
                sequence: RigObservationSequence::new(3).unwrap(),
            },
            observed_at: RigObservationTimestamp::new(1_721_798_400_000, 9_000).unwrap(),
            fields: RigObservationFields {
                dial_frequency: RigReadback::Observed(DialFrequency::from_hz(14_074_000).unwrap()),
                modulation: RigReadback::Observed(RadioModulation::DataUpperSideband),
                passband: RigReadback::Observed(RadioPassband::from_hz(3_000).unwrap()),
                vfo: RigReadback::Observed(RigVfo::A),
                split: RigReadback::Observed(SplitReadback::new(false, None)),
                power: RigReadback::Unavailable,
                ptt_asserted: RigReadback::Unsupported,
            },
        };
        let json = serde_json::to_string(&observation).unwrap();
        assert!(json.contains(r#""power":{"status":"unavailable"}"#));
        assert!(json.contains(r#""ptt_asserted":{"status":"unsupported"}"#));
        assert!(!json.contains(r#""power":0"#));
        assert!(!json.contains(r#""ptt_asserted":false"#));
        assert_eq!(
            serde_json::from_str::<RigObservation>(&json).unwrap(),
            observation
        );
    }

    #[test]
    fn freshness_checks_both_timelines_without_sleeping() {
        let observation = RigObservation {
            profile_revision_id: "prv_01jrigprofile".parse().unwrap(),
            provenance: RigObservationProvenance {
                service_instance_id: "svc_01jrigprocess".parse().unwrap(),
                connection_generation: RigConnectionGeneration::new(1).unwrap(),
                sequence: RigObservationSequence::new(1).unwrap(),
            },
            observed_at: RigObservationTimestamp::new(1_000, 10).unwrap(),
            fields: RigObservationFields {
                dial_frequency: RigReadback::StaleOrUnverified,
                modulation: RigReadback::StaleOrUnverified,
                passband: RigReadback::StaleOrUnverified,
                vfo: RigReadback::StaleOrUnverified,
                split: RigReadback::StaleOrUnverified,
                power: RigReadback::StaleOrUnverified,
                ptt_asserted: RigReadback::StaleOrUnverified,
            },
        };
        let policy = RigFreshnessPolicy::new(100, 5).unwrap();
        let age = observation
            .fresh_at(
                ClockSample {
                    utc: UtcInstant::from_unix_millis(1_050).unwrap(),
                    monotonic: MonotonicInstant::from_millis(60),
                },
                policy,
            )
            .unwrap();
        assert_eq!(age.utc_millis(), 50);
        assert_eq!(age.monotonic_millis(), 50);
        assert!(matches!(
            observation.fresh_at(
                ClockSample {
                    utc: UtcInstant::from_unix_millis(1_200).unwrap(),
                    monotonic: MonotonicInstant::from_millis(210),
                },
                policy,
            ),
            Err(RigFault::Stale { .. })
        ));
        assert!(matches!(
            observation.fresh_at(
                ClockSample {
                    utc: UtcInstant::from_unix_millis(1_050).unwrap(),
                    monotonic: MonotonicInstant::from_millis(80),
                },
                policy,
            ),
            Err(RigFault::ContradictoryTiming { .. })
        ));
    }

    #[test]
    fn serialized_contracts_cannot_bypass_checked_bounds() {
        let duplicate_capability = r#"{"generation":1,"evidence":[{"capability":"power","status":"runtime_probed"},{"capability":"power","status":"physically_verified"}]}"#;
        assert!(serde_json::from_str::<RigCapabilityReport>(duplicate_capability).is_err());
        assert!(
            serde_json::from_str::<RigObservationTimestamp>(
                r#"{"utc_unix_millis":-1,"monotonic_millis":0}"#
            )
            .is_err()
        );
        let findings = (0..=MAX_RIG_VALIDATION_FINDINGS)
            .map(|_| r#"{"capability":"power","status":"unsupported","disposition":"failed"}"#)
            .collect::<Vec<_>>()
            .join(",");
        let oversized =
            format!(r#"{{"profile_revision_id":"prv_01jrigprofile","findings":[{findings}]}}"#);
        assert!(serde_json::from_str::<RigProfileValidation>(&oversized).is_err());
    }
}
