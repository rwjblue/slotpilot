# Architecture

## System shape

SlotPilot is a local service with multiple clients, not a desktop application with embedded radio logic.

```text
┌─────────────────────┐          ┌─────────────────────┐
│ SlotPilot Desktop   │          │ slotpilot CLI       │
│ macOS/Windows/Linux │          │ human or automation │
└──────────┬──────────┘          └──────────┬──────────┘
           │ versioned commands/results/events         │
           └──────────────────────┬─────────────────────┘
                                  ▼
                         ┌──────────────────┐
                         │   slotpilotd     │
                         │ sole live owner  │
                         └────────┬─────────┘
                                  │
        ┌────────────┬────────────┼─────────────┬─────────────┐
        ▼            ▼            ▼             ▼             ▼
      audio       rigctld      FT8/WSPR       SQLite       integrations
      CPAL        Hamlib       protocol       + outbox     ADIF/WSPRnet/
                                                             AntennaBench
```

The daemon owns:

- input and output streams;
- Hamlib/rigctld connection;
- PTT and transmit scheduling;
- current radio/audio verification state;
- UTC/monotonic slot clock;
- FT8 caller queue and QSO state machine;
- WSPR receive/transmit coordinator;
- profile selection for the active session;
- authoritative SQLite connection and outboxes.

A client that disconnects does not become a new hardware owner. A client may observe state and submit commands according to API permissions and current operator authority.

## Planned workspace

The following is a target decomposition, not an instruction to create all crates in the first change:

```text
crates/
  domain/         callsigns, bands, frequencies, IDs, profiles, QSO values
  protocol/       SlotPilot traits plus mfsk-core FT8/WSPR adapter
  audio/          CPAL adapters, resampling, timeline, waterfall data
  rig/            rigctld/Hamlib client, capabilities, model quirks
  policy/         caller queue, duplicate rules, lane/parity planners
  operations/     FT8/WSPR coordinators, slot clock, TxSupervisor
  storage/        SQLite, migrations, event history, durable outbox
  logging/        log-sink trait, ADIF import/export
  integrations/   WSPRnet, WSJT-X compatibility, external adapters
  api/            versioned commands, results, events, snapshots
  ipc/            local transport server/client
  testkit/        virtual clock, fake rig/audio/protocol, replay fixtures
apps/
  daemon/         slotpilotd
  cli/            slotpilot
  desktop/        native desktop client
```

The bootstrapped workspace contains `domain`, `api`, `ipc`, `operations`,
`storage`, `testkit`, `slotpilotd`, and `slotpilot`. Their allowed internal dependency
graph is:

```text
slotpilotd ─┐
            ├──> api ──> domain
slotpilot  ─┘

slotpilotd ─┐
slotpilot  ─┴──> ipc ──> api

operations ──> domain
testkit ─────> operations ──> domain
storage ────────────────────> domain
```

`domain` has no internal workspace dependency. `api` may depend on `domain`;
the daemon and CLI composition shells may depend on `api`. The executable
`mise run check-dependencies` task verifies this allow-list from Cargo's
resolved graph and is part of `mise run ci`. Later focused issues may extend
the graph only in the direction described below.

### Dependency direction

- `domain` depends on no infrastructure or application crate.
- API wire types depend on stable domain representations, not GUI or database objects.
- protocol, audio, rig, policy, storage, logging, and integrations implement boundaries used by operations.
- operations coordinates domain behavior and ports; it does not import desktop or CLI types.
- apps compose implementations and may use `anyhow` at the boundary.
- clients depend on API/IPC, not on rig, audio, storage, or operations internals.

Circular dependencies should be treated as a sign that a boundary or shared type is misplaced.

## Command and event model

Commands are submitted in a versioned envelope:

```rust
pub struct CommandEnvelope {
    pub api_version: u32,
    pub request_id: RequestId,
    pub command: Command,
    pub external_context: Option<ExternalContext>,
}
```

The service returns a bounded result and publishes ordered events. Mutating request IDs are durable enough to distinguish:

- a safe retry of the same operation;
- a conflicting reuse of an identity;
- a newly requested operation.

Clients obtain a snapshot before following events so they can recover from disconnection without reconstructing all history.

The initial wire format is versioned JSON. Streaming output uses one JSON value per line. Domain code must not parse display strings to infer behavior.

## Local IPC

Expected transports:

- Unix-domain socket on macOS and Linux;
- named pipe on Windows;
- loopback TCP only for explicit development/test scenarios.

The endpoint is machine-local and user-scoped by default. Filesystem/socket permissions and peer identity should be validated before privileged commands are admitted.

## Protocol boundary

`mfsk-core` is the planned initial GPL-compatible FT8/WSPR implementation. It remains behind SlotPilot-owned traits because it is a young `0.x` dependency and must not define public APIs or persisted representations.

The protocol layer returns typed decodes and messages. It does not select callers, log QSOs, control PTT, or decide whether a message may advance an exchange.

A reference-process or fixture-based adapter may be used only in testing to compare known behavior.

## Audio boundary

CPAL is the planned cross-platform stream layer. Real-time callbacks move samples to and from bounded preallocated queues. Resampling, FFTs, decode work, logging, and rig interaction occur elsewhere.

A timestamped audio timeline maps device samples into protocol windows. Transmit waveforms are fully prepared before their slot deadline and placed at a deterministic sample position.

Platform adapters provide stable device identities and permission/setup behavior.

## Time boundary

The slot clock samples UTC and monotonic time together, calculates future protocol boundaries, and schedules against monotonic deadlines. It periodically checks the mapping and exposes clock health.

Median decoded `DT` may inform diagnostics but does not independently grant transmit authority.

All time-dependent operations receive an injected clock abstraction. Tests must be able to advance slots without sleeping in real time.

## Rig boundary

The first backend uses one persistent connection to `rigctld`. It probes capabilities instead of assuming that split, PTT, mode, filter, power, or meter operations exist.

Rig profiles distinguish:

- CAT endpoint and model;
- PTT method;
- digital-mode mapping;
- split/Fake-It capability;
- PTT lead/tail timings;
- maximum power and verification rules;
- radio-specific quirks.

A narrowly scoped direct-radio adapter may be added only for a documented gap and must not bypass the common safety and capability model.

## Operations

### FT8 coordinator

The coordinator owns run policy, caller queue, and QSO state. A QSO state transition requires a typed message whose calls, stage, and parity match the active attempt.

Completion commits the QSO, transcript, duplicate update, and log outbox atomically before the queue advances.

### WSPR coordinator

WSPR receive, spot storage/upload, and transmit schedules are independent of FT8 QSO state. FT8 and WSPR compete for the same transmit supervisor and cannot overlap.

### Transmit supervisor

Only the transmit supervisor may key PTT. It validates an immutable plan against current authority, clock, rig, audio, profile, power, band/mode, and collision state. A separate watchdog can dekey independently.

## Storage

SQLite is authoritative for profiles, sessions, commands, events, decodes, QSO attempts, QSOs, WSPR spots/transmissions, rule evaluations, and outboxes.

Persistence should favor explicit schema versions and forward migrations. The database should not persist live transmit authority.

An outbox transactionally couples a committed domain record with pending external side effects. A successful external receipt is stored separately so restart can safely continue.

## Recovery

On daemon restart:

- force or request PTT off before recovering ordinary state;
- do not restore attended transmit authority;
- retain incomplete QSO attempts as inactive diagnostic history;
- permit local receive monitoring only according to explicit startup configuration;
- resume safe external outbox delivery;
- require explicit re-arming for future transmissions unless a later accepted decision defines a narrowly bounded alternative.

## Desktop framework

A Rust-native desktop client such as `eframe`/`egui` is the current preference, but the daemon boundary makes the choice replaceable. Framework selection should be confirmed in a scoped issue before desktop implementation begins.
