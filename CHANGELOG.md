# Changelog

SlotPilot follows semantic versions. Until an end-user release is explicitly
authorized, development versions use `0.y.z-dev.N`, remain unpublished, and
make no compatibility promise beyond the reviewed schema and wire fixtures.

## 0.1.0-dev.0 — 2026-07-23

- Established the Phase 0 Rust workspace and typed domain vocabulary.
- Added versioned no-op command, snapshot, event, cursor, and error contracts.
- Added user-scoped local IPC, durable idempotency, initial SQLite storage,
  deterministic time seams, and hardware-free test ports.
- Added an unsigned local development build and no-op handshake workflow.

RF operation is unavailable. This version cannot operate a radio and has no
audio, FT8, WSPR, logging, station-control, transmit, or desktop behavior.
