//! Reserved infrastructure boundary for read-only rig adapters.
//!
//! Phase 3.1 intentionally provides no network, process, serial, radio, or
//! hardware implementation here. A later focused issue may implement the
//! consumer-owned [`slotpilot_operations::ReadOnlyRigPort`] using only
//! SlotPilot-owned [`slotpilot_domain::RigProfile`] values. This crate must not
//! add rig mutation, raw commands, PTT, audio output, transmit authority, or
//! scheduling.
