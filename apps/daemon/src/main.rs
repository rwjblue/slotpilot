//! Compile-only composition shell for the SlotPilot station daemon.
//!
//! The daemon is the sole future owner of live station state and hardware.
//! Phase 0.1 intentionally performs no startup, I/O, scheduling, persistence,
//! protocol, audio, rig, or transmit work.

fn main() -> anyhow::Result<()> {
    Ok(())
}
