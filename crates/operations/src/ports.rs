//! Consumer-owned ports for future hardware and protocol adapters.

use slotpilot_audio::{CaptureBatch, InputFault, InputHealth};
use slotpilot_domain::TransmissionId;
use slotpilot_protocol::{Ft8Decode, Ft8WaveformError, Ft8WaveformRequest, PcmBuffer};
use thiserror::Error;

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
