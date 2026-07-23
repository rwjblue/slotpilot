# `crossbeam-queue` 0.3.13 review

- Review date: 2026-07-23
- Selected crate: `crossbeam-queue =0.3.13`
- Crates.io archive SHA-256:
  `803d13fb3b09d88be9f4dbc29062c66b19bf7170867ceb746d2a8689bf6c7a26`
- Upstream tag commit:
  `9b56303b8aa9ff8ec5bbebb9d2da05e034977889`
- Enabled features: `std` (and its `alloc` prerequisite)
- Default features: disabled

This is the maintained Phase 2 adoption record for the bounded receive-input
callback queue. It authorizes only `ArrayQueue` inside the private
`slotpilot-audio` capture adapter. It does not authorize unbounded queues,
blocking synchronization, output audio, persistence, protocol decode, client
state, rig control, PTT, scheduling, transmit, or RF.

## Selection and closure

Version 0.3.13 was the current published release when Phase 2.3 began. The
workspace exact-pins it and enables only `std`; the locked closure adds
`crossbeam-utils`. Both crates declare MIT OR Apache-2.0, compatible with
SlotPilot's GPL-3.0-or-later distribution. The crate requires Rust 1.60, below
the repository's exact Rust 1.97.1 toolchain.

An OSV ecosystem query for `crossbeam-queue` 0.3.13 on 2026-07-23 returned no
advisory. The exact archive checksum and release-tag commit are recorded above.
Upgrades require a focused issue, refreshed license/advisory/source review,
callback stress tests, and the complete cross-platform gate.

## Real-time boundary

The capture adapter constructs fixed-capacity `ArrayQueue` instances and every
sample buffer before opening a stream. The callback uses only nonblocking
`pop`, `push`, bounded sample conversion/copy, and atomic counters. It never
grows a vector or allocates queue nodes. Worker-side consumption copies a
delivered batch before returning the fixed buffer to the free pool.

When the pool is empty, the callback drops the new frames, advances the source
position, increments an overflow counter, emits a bounded typed fault when
space remains, and attaches a discontinuity with the known dropped-frame count
to the next delivered batch. It never waits for a consumer or overwrites an
older batch.

The workspace forbids unsafe code in SlotPilot sources. The dependency uses
reviewed internal unsafe code and atomics to implement its concurrent queue;
no dependency type appears in public signatures. Dependency and marked-source
guards keep imports and queue operations inside the private capture adapter.
