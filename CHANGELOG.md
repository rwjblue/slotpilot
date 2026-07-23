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
- Added the bounded Phase 1 offline FT8 harness: owned typed outcomes, exact
  reviewed protocol dependency, redistributable fixtures, message conformance,
  deterministic PCM synthesis, bounded RIFF/WAVE parsing, and reproducible
  static-recording decode.
- Added bounded Phase 2 receive foundations: exact input discovery/capture,
  deterministic resampling and slot/clock gating, a bounded spectrum model,
  and SQLite schema version 2 for atomic receive diagnostics/classifications.

RF operation is unavailable. Offline FT8 library tests do not provide live
audio or station behavior. This version cannot operate a radio and has no
audio-device, WSPR, logging, station-control, transmit, or desktop behavior.
