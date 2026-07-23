//! Deterministic operating-coordination contracts.
//!
//! Phase 0 provides only clock sampling, UTC/monotonic mapping, slot
//! arithmetic, health observation, and virtual time. It contains no sleeping,
//! scheduler side effect, authority grant, waveform, audio, rig, or PTT path.

mod time;

pub use time::{
    Clock, ClockFault, ClockHealth, ClockMonitor, ClockSample, MonotonicDeadline, MonotonicInstant,
    SlotTimeError, UtcInstant, VirtualClock,
};
