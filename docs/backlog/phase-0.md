# Phase 0 backlog — workspace and contracts

These tasks are written so they can become GitHub issues. They are ordered but some may proceed in parallel after the first task.

## 0.1 Establish the Rust workspace and boundary shells

**Goal:** create the minimal compile-only workspace that enforces intended dependency direction.

In scope:

- create only the crates/apps needed for Phase 0;
- establish workspace dependency/lint conventions;
- add CI for format, lint, tests, and documentation links or required-file checks;
- add empty public module documentation explaining each boundary;
- add a dependency-direction check or documented review mechanism.

Out of scope:

- protocol algorithms;
- CPAL or Hamlib connection;
- SQLite behavior beyond what a later issue owns;
- desktop framework;
- any PTT or waveform generation.

Acceptance:

- [ ] workspace builds on macOS, Windows, and Linux CI;
- [ ] `cargo fmt`, `cargo clippy`, and `cargo test` pass;
- [ ] library crates use typed errors and no `anyhow` in public APIs;
- [ ] no physical-hardware dependency exists;
- [ ] crate responsibilities match `docs/architecture.md`.

## 0.2 Define domain identifiers and value types

**Goal:** define stable types used across commands, events, storage, and operations.

Candidate types:

- request, command, event, session, profile-revision, QSO, QSO-attempt, and transmission IDs;
- callsign/full call/base-call distinction;
- band, dial frequency, audio frequency, power, UTC slot, and mode;
- station/operator/owner identities.

Acceptance:

- [ ] parsing and validation are explicit;
- [ ] display and wire representations have fixtures;
- [ ] invalid values return typed errors;
- [ ] normalization never destroys the full original callsign;
- [ ] no GUI/storage framework types leak into domain APIs.

## 0.3 Define API version negotiation and no-op snapshot

**Goal:** allow a CLI client and daemon to negotiate a schema and retrieve an empty station snapshot.

In scope:

- command envelope;
- result and error envelope;
- API capabilities/version response;
- bounded snapshot with explicit “not configured/not running” states;
- JSON fixtures.

Acceptance:

- [ ] incompatible versions fail with a stable error code;
- [ ] additive unknown fields are tolerated according to documented rules;
- [ ] snapshots are bounded and deterministic;
- [ ] CLI table and JSON output use the same result model.

## 0.4 Implement request-ID idempotency semantics

**Goal:** make uncertain retries safe before any hardware side effect exists.

Acceptance:

- [ ] same request ID and same canonical command returns original result;
- [ ] same request ID and different command returns `request_id_conflict`;
- [ ] persistence/restart behavior is tested;
- [ ] commands distinguish read-only from mutating behavior;
- [ ] no side-effect implementation is required.

## 0.5 Establish local IPC adapters

**Goal:** connect daemon and CLI over platform-appropriate local transport.

In scope:

- Unix-domain socket adapter for macOS/Linux;
- named-pipe adapter contract and implementation for Windows;
- explicit loopback development adapter if needed;
- framing, size limits, cancellation, and graceful disconnect.

Acceptance:

- [ ] endpoint is local/user-scoped by default;
- [ ] oversized and malformed messages are rejected without daemon failure;
- [ ] client reconnect can request a fresh snapshot;
- [ ] no transport grants transmit authority;
- [ ] platform tests pass in CI.

## 0.6 Add virtual clock and slot arithmetic

**Goal:** establish deterministic synchronized-mode scheduling primitives without real sleeping.

In scope:

- UTC/monotonic mapping abstraction;
- FT8 15-second and WSPR two-minute boundary calculations;
- clock-health state and jump detection contract;
- virtual clock test utility.

Acceptance:

- [ ] tests cover exact boundaries, late calls, UTC midnight, and clock jumps;
- [ ] production code can schedule against monotonic deadlines;
- [ ] no audio or transmit implementation exists;
- [ ] authority expiry can be tested with virtual time.

## 0.7 Define rig, audio, protocol, and transmit-supervisor ports plus fakes

**Goal:** make operations code testable before physical adapters.

Acceptance:

- [ ] fake rig can inject connect, readback mismatch, command rejection, and PTT-stuck states;
- [ ] fake audio can inject device loss, overrun, underrun, clipping, and latency;
- [ ] fake protocol can emit typed messages and deterministic waveforms;
- [ ] transmit-supervisor port has an emergency-unkey path;
- [ ] no fake can accidentally access real hardware.

## 0.8 Define initial SQLite schema and migrations

**Goal:** establish durable identities, schema versioning, commands/events, profiles, and outbox concepts.

In scope:

- schema/migration harness;
- profile revisions and session-context snapshots;
- accepted command identities/results;
- append-only or ordered operational events;
- generic outbox/receipt tables.

Out of scope:

- final FT8/QSO/WSPR field set;
- ADIF export;
- network upload.

Acceptance:

- [ ] clean creation and migration tests pass;
- [ ] duplicate request IDs are constrained appropriately;
- [ ] no transmit authority is persisted;
- [ ] crash/reopen tests preserve accepted command results;
- [ ] database errors are typed at the library boundary.

## 0.9 Add event subscription and replay cursor contract

**Goal:** let CLI/desktop clients observe daemon state without polling every subsystem.

Acceptance:

- [ ] ordered event envelope and cursor semantics are documented;
- [ ] slow consumers have explicit bounded behavior;
- [ ] disconnect/reconnect behavior is tested;
- [ ] unknown event kinds are surfaced safely;
- [ ] events contain no dependency-specific implementation types.

## 0.10 Document and automate the first development release process

**Goal:** make local-agent work reproducible without producing an end-user radio release.

In scope:

- development build instructions;
- schema/fixture compatibility checks;
- changelog convention;
- unsigned local artifacts only;
- no on-air claims.

Acceptance:

- [ ] a new contributor can build and run the no-op daemon/CLI handshake;
- [ ] repository status clearly states that RF operation is unavailable;
- [ ] release process does not publish unsafe or misleading binaries.
