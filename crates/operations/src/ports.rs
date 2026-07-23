//! Consumer-owned ports for future hardware and protocol adapters.

use slotpilot_audio::{CaptureBatch, InputFault, InputHealth};
use slotpilot_domain::{DialFrequency, OperatingMode, Power, TransmissionId};
use slotpilot_protocol::{Ft8Decode, Ft8WaveformError, Ft8WaveformRequest, PcmBuffer};
use thiserror::Error;

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

/// Receive-only audio boundary consumed by operations outside the callback.
pub trait ReceiveAudioPort {
    /// Returns current owned input health or the next deterministic fault.
    fn health(&mut self) -> Result<InputHealth, InputFault>;
    /// Drains one already-bounded batch without blocking.
    fn next_batch(&mut self) -> Result<Option<CaptureBatch>, InputFault>;
}

/// Protocol boundary consumed by operations.
pub trait ProtocolPort {
    /// Drains deterministically ordered owned FT8 decodes.
    fn drain_decodes(&mut self) -> Vec<Ft8Decode>;
    /// Returns deterministic offline samples for an owned FT8 request.
    fn prepare_waveform(
        &mut self,
        request: &Ft8WaveformRequest,
    ) -> Result<PcmBuffer, Ft8WaveformError>;
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
