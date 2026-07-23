//! Consumer-owned ports for future hardware and protocol adapters.

use slotpilot_domain::{DialFrequency, FullCallsign, OperatingMode, Power, TransmissionId};
use thiserror::Error;

use crate::MonotonicInstant;

/// Snapshot of verified rig state using only SlotPilot-owned values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RigState {
    /// Verified dial frequency.
    pub dial_frequency: DialFrequency,
    /// Verified synchronized operating mode.
    pub mode: OperatingMode,
    /// Verified configured power.
    pub power: Power,
    /// Whether PTT readback is asserted.
    pub ptt_asserted: bool,
}

/// Narrow Phase 0 rig command vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RigCommand {
    /// Request a dial frequency; a future adapter must verify readback.
    SetDialFrequency(DialFrequency),
    /// Request a mode; a future adapter must verify readback.
    SetMode(OperatingMode),
}

/// Typed rig failures required by operations fault handling.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RigFault {
    /// The rig connection is unavailable.
    #[error("rig disconnected")]
    Disconnected,
    /// Readback is older than the caller's freshness requirement.
    #[error("rig readback is stale")]
    StaleReadback,
    /// Readback conflicts with the requested state.
    #[error("rig readback contradicts requested state")]
    ContradictoryReadback {
        /// State requested by operations.
        expected: RigState,
        /// State reported by the adapter.
        observed: RigState,
    },
    /// The adapter rejected a command without applying it.
    #[error("rig command rejected")]
    CommandRejected,
    /// The rig moved outside a SlotPilot request.
    #[error("rig moved unexpectedly")]
    UnexpectedMovement(RigState),
    /// PTT remains asserted after an unkey request.
    #[error("rig PTT remains asserted")]
    PttStuck,
}

/// Rig-control boundary consumed by operations.
pub trait RigPort {
    /// Returns verified current state or a typed failure.
    fn read_state(&mut self) -> Result<RigState, RigFault>;
    /// Applies one narrow command and returns verified resulting state.
    fn apply(&mut self, command: RigCommand) -> Result<RigState, RigFault>;
}

/// Observable audio health using no device-library types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioHealth {
    /// Estimated end-to-end latency.
    pub latency_millis: u32,
    /// Estimated sample-clock drift.
    pub drift_parts_per_million: i32,
}

/// Typed audio failure injected or reported at a monotonic instant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("audio fault at {occurred_at:?}: {kind:?}")]
pub struct AudioFault {
    /// Process-local occurrence time.
    pub occurred_at: MonotonicInstant,
    /// Specific fault kind.
    pub kind: AudioFaultKind,
}

/// Audio failures relevant to future operations admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioFaultKind {
    /// Configured device disappeared.
    DeviceLost,
    /// Input producer overran its bounded queue.
    Overrun,
    /// Output consumer underrun occurred.
    Underrun,
    /// Samples exceeded the configured clipping threshold.
    Clipping,
    /// Sample clock drift crossed its configured bound.
    Drift {
        /// Observed signed drift.
        parts_per_million: i32,
    },
    /// Measured latency crossed its configured bound.
    Latency {
        /// Observed latency.
        millis: u32,
    },
    /// Callback execution was delayed beyond its configured bound.
    CallbackDelay {
        /// Observed callback delay.
        millis: u32,
    },
}

/// Audio-health boundary consumed by operations.
pub trait AudioPort {
    /// Returns current health or the next deterministic fault.
    fn health(&mut self) -> Result<AudioHealth, AudioFault>;
}

/// Typed protocol message independent of any implementation dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolMessage {
    /// Synchronized protocol mode.
    pub mode: OperatingMode,
    /// Resolved transmitting station.
    pub sender: FullCallsign,
    /// Resolved intended recipient.
    pub recipient: FullCallsign,
    /// Typed message meaning.
    pub payload: ProtocolPayload,
}

/// Small typed message vocabulary sufficient for Phase 0 fakes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolPayload {
    /// Signal report in decibels.
    SignalReport(i8),
    /// Final acknowledgement.
    Acknowledgement,
}

/// A message observed at a deterministic monotonic time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMessage {
    /// Observation time.
    pub observed_at: MonotonicInstant,
    /// Resolved typed message.
    pub message: ProtocolMessage,
}

/// Prepared in-memory placeholder samples with no audio-device behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    /// Signed normalized placeholder samples.
    pub samples: Vec<i16>,
}

/// Protocol boundary consumed by operations.
pub trait ProtocolPort {
    /// Drains deterministic typed decodes.
    fn drain_decodes(&mut self) -> Vec<DecodedMessage>;
    /// Returns deterministic placeholder samples for a typed message.
    fn prepare_waveform(&mut self, message: &ProtocolMessage) -> Waveform;
}

/// Typed reason the logical transmit-supervisor port refuses admission.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TxInhibition {
    /// No explicit unexpired authority exists.
    #[error("transmit authority is missing or expired")]
    MissingAuthority,
    /// Clock health is unsuitable for synchronized work.
    #[error("clock is unhealthy")]
    ClockUnhealthy,
    /// Rig state is unsuitable or unverified.
    #[error("rig is unavailable or unverified")]
    RigUnavailable,
    /// Audio state is unsuitable or unverified.
    #[error("audio is unavailable or unverified")]
    AudioUnavailable,
    /// Another logical transmission conflicts with this identity.
    #[error("transmission conflicts with {0}")]
    Conflict(TransmissionId),
    /// Emergency-stop inhibition is latched.
    #[error("emergency stop is latched")]
    EmergencyStopLatched,
}

/// Failure of the logical emergency-unkey request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EmergencyUnkeyError {
    /// The fake or future adapter reports PTT stuck asserted.
    #[error("logical PTT state remains asserted")]
    PttStuck,
}

/// Sole logical admission and emergency-unkey boundary for transmission.
pub trait TransmitSupervisorPort {
    /// Checks whether a logical transmission identity may be admitted.
    fn admit(&mut self, transmission_id: &TransmissionId) -> Result<(), TxInhibition>;
    /// Bypasses ordinary admission to request immediate unkey.
    fn emergency_unkey(&mut self) -> Result<(), EmergencyUnkeyError>;
}
