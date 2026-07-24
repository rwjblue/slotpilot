//! Deterministic operating-coordination contracts.
//!
//! This crate provides clock sampling, UTC/monotonic mapping, slot arithmetic,
//! virtual time, receive clock-health/alignment gating, and daemon-facing
//! ports. It contains no sleeping, scheduler side effect, authority grant,
//! waveform, audio output, rig implementation, or PTT path.

mod ports;
mod receive_clock;
mod rig;
mod time;

pub use ports::{
    EmergencyUnkeyError, ProtocolPort, ReceiveAudioPort, TransmitSupervisorPort, TxInhibition,
};
pub use receive_clock::{
    ClockGatedTimelineEvent, ClockProcessGeneration, DEFAULT_CLOCK_FRESHNESS_MILLIS,
    DEFAULT_CLOCK_JUMP_TOLERANCE_MILLIS, DEFAULT_CLOCK_RECOVERY_SAMPLES,
    DEFAULT_CLOCK_SAMPLE_CADENCE_MILLIS, DEFAULT_CLOCK_SAMPLE_GAP_MILLIS,
    DEFAULT_CLOCK_SAMPLING_DELAY_MILLIS, GenerationClockSample, ReceiveClockConfig,
    ReceiveClockDriver, ReceiveClockError, ReceiveClockFault, ReceiveClockMonitor,
    ReceiveClockPoll, ReceiveClockSnapshot, ReceiveClockSource, ReceiveClockState,
    ReceiveClockTransition, ReceiveWindowAlignment, SystemClock,
};
pub use rig::{
    MAX_RIG_CAPABILITIES, MAX_RIG_VALIDATION_FINDINGS, ReadOnlyRigPort, RigCapability,
    RigCapabilityEvidence, RigCapabilityReport, RigCapabilityStatus, RigConnectionGeneration,
    RigContractError, RigFault, RigFaultKind, RigFreshnessPolicy, RigLifecycleState,
    RigObservation, RigObservationAge, RigObservationFields, RigObservationProvenance,
    RigObservationSequence, RigObservationTimestamp, RigObservedField, RigOperation,
    RigProfileValidation, RigReadback, RigValidationDisposition, RigValidationFinding,
    validate_profile_capabilities,
};
pub use time::{
    Clock, ClockFault, ClockHealth, ClockMonitor, ClockSample, MonotonicDeadline, MonotonicInstant,
    SlotTimeError, UtcInstant, VirtualClock,
};
