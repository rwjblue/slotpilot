//! Deterministic operating-coordination contracts.
//!
//! Phase 0 provides only clock sampling, UTC/monotonic mapping, slot
//! arithmetic, health observation, and virtual time. It contains no sleeping,
//! scheduler side effect, authority grant, waveform, audio, rig, or PTT path.

mod ports;
mod time;

pub use ports::{
    EmergencyUnkeyError, ProtocolPort, ReceiveAudioPort, RigCommand, RigFault, RigPort, RigState,
    TransmitSupervisorPort, TxInhibition,
};
pub use time::{
    Clock, ClockFault, ClockHealth, ClockMonitor, ClockSample, MonotonicDeadline, MonotonicInstant,
    SlotTimeError, UtcInstant, VirtualClock,
};
