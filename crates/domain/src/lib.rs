//! Stable, validated domain vocabulary owned by SlotPilot.
//!
//! IDs use explicit lowercase prefixes and bounded ASCII payloads. Callsigns
//! retain their exact full spelling while exposing a separate normalized base
//! call for policies that explicitly need it. Radio values use integer units,
//! avoiding floating-point ambiguity at API and persistence boundaries.

mod callsign;
mod ids;
mod radio;

pub use callsign::{
    BaseCallsign, CallsignError, FullCallsign, OperatorCallsign, OwnerCallsign, StationCallsign,
};
pub use ids::{
    CommandId, EventId, IdError, ProfileRevisionId, QsoAttemptId, QsoId, RequestId,
    ServiceInstanceId, SessionId, TransmissionId,
};
pub use radio::{
    AudioFrequency, Band, DialFrequency, OperatingMode, Power, RadioValueError, UtcSlot,
};
