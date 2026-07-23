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

The workspace contains `domain`, `api`, `ipc`, `protocol`, `audio`,
`operations`, `storage`, `testkit`, `slotpilotd`, and `slotpilot`. Their
current allowed internal dependency graph is:

```text
slotpilotd ─┐
            ├──> api ──> domain
slotpilot  ─┘

slotpilotd ─┐
slotpilot  ─┴──> ipc ──> api

protocol ─────> domain
audio
operations ───> audio
operations ───> protocol ───> domain
testkit ──────> audio
testkit ──────> operations ──> protocol ──> domain
storage ─────> audio
storage ─────> protocol ───> domain
storage ───────────────────> domain
```

`domain` and the owned `audio` contract have no internal workspace dependency.
`protocol` may depend on `domain`; `operations` may depend on `audio`,
`protocol`, and `domain`. Storage may depend on owned `audio`, `protocol`, and
`domain` values solely to validate and reconstruct durable records; it cannot
import private device/FFT/protocol dependencies or operations/API types.
`api` may depend on `domain`; the daemon and CLI composition shells may depend
on `api`. The executable `mise run
check-dependencies` task verifies this allow-list from Cargo's resolved graph
and is part of `mise run ci`. Later focused issues may extend the graph only in
the direction described below.

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

`slotpilot-protocol` owns the FT8 message classification, message-codec,
offline decode metadata, and PCM/waveform contracts. Supported resolved
messages are structurally distinct from unresolved hashes, unsupported
structured content, ambiguous data, and free text; consumers must use an
explicit checked conversion to obtain a resolved message.

The exact reviewed `mfsk-core` 0.7.4 pin is the initial GPL-compatible offline
FT8 implementation. Defaults are disabled and WSPR is not enabled; its later
protocol work remains separately scoped. The dependency remains behind
SlotPilot-owned traits because it is a young `0.x` dependency and must not
define public APIs or persisted representations. The review is maintained in
`dependencies/mfsk-core-0.7.4.md`.

The protocol layer returns typed decodes, messages, packed bits, and bounded
offline PCM with explicit sample rate, channels, sample format, duration,
amplitude, audio frequency, and placement units. Result ordering is defined by
owned metadata and canonical text rather than adapter worker order. It does
not select callers, log QSOs, control PTT, or decide whether a message may
advance an exchange.

Phase 1 waveform synthesis accepts only resolved supported messages and emits
canonical mono signed 16-bit PCM at 12,000 Hz. One FT8 frame is 151,680 samples
(12.64 seconds). Frame-only output is distinct from explicit placement within a
180,000-sample (15-second) offline slot. The in-memory RIFF/WAVE helper only
serializes a checked buffer; it does not write, select, or play an audio device.

The offline decoder accepts exactly one canonical 180,000-sample slot. Its
bounded RIFF/WAVE parser requires uncompressed signed 16-bit PCM, validates
declared byte rates and block alignment, skips bounded unknown chunks, and
rejects missing, duplicate, truncated, or oversized content before decoding.
The decoder returns owned integer time, frequency, and SNR metadata, removes
only identical normalized results, and applies the protocol crate's stable
ordering. Empty silence is a successful empty result, not an invented decode.

A reference-process or fixture-based adapter may be used only in testing to compare known behavior.

## Audio boundary

CPAL 0.18.1 is the reviewed cross-platform audio layer, pinned exactly with
optional and default features disabled. Its private receive discovery adapter
enumerates the standard Core Audio, WASAPI, or ALSA host, maps supported input
configuration ranges into owned values, and uses CPAL's host-qualified stable
device ID for exact lookup. It never calls a default-device selector. A
missing identity, permission denial, host failure, device disappearance,
unsupported configuration, or empty input set remains a distinct typed
result. The maintained dependency review is
[`dependencies/cpal-0.18.1.md`](dependencies/cpal-0.18.1.md).

The private input adapter opens only an exact selected stable identity and
configuration. It preallocates a bounded lock-free pool and queue before the
stream starts. The callback selects one source channel, converts supported PCM
representations into mono signed-16-bit samples, records capture position and
timing, updates atomic health counters, and moves preallocated storage without
allocation or waiting. Overflow drops new frames, advances the source
position, and marks the next delivered batch with the exact known gap.
Device/backend failure stops further callback processing and never selects a
replacement.

Resampling, FFTs, decode work, persistence, client state, logging, and rig
interaction occur elsewhere.

The worker-side receive timeline uses owned fixed-point rational interpolation
with source-frame position and paired UTC/monotonic evidence. It holds at most
one canonical FT8 window plus one source sample, maps output positions at exact
12 kHz ticks, and emits only a complete 180,000-sample window beginning at a
typed 15-second UTC slot. Partial slots are observable and withheld. Gaps,
overlap, out-of-order data, explicit discontinuity, late arrival, excessive
jitter/drift, UTC remapping, or generation/configuration change reset bounded
resampler state and cannot produce a valid window. Detailed tolerances and
reset behavior are in [`audio-timeline.md`](audio-timeline.md).

Transmit waveforms are fully prepared before their slot deadline and placed at
a deterministic sample position.

Platform adapters provide stable device identities and permission/setup
behavior. Discovery may enumerate input devices but cannot select by display
name. Capture lookup uses the same exact identity and never consults a default
device or same-named replacement.

The owned Phase 2 contract remains independent of device-library types. Stable
input identity is an opaque platform value structurally separate from display
metadata; display names can never select or recover a device. Checked input
configurations record sample rate, channel count and selection, and source
sample format. Bounded normalized mono batches carry process/stream generations,
monotonic source-frame positions, paired UTC/monotonic evidence, callback
diagnostics, and explicit discontinuities. A checked canonical FT8 receive
window is exactly 180,000 mono signed-16-bit samples at 12,000 Hz aligned to a
15-second UTC boundary, matching the offline decoder without importing
protocol implementation types.

Constructing owned batches and copying samples out of the reusable pool is
worker-side work. The live callback only converts/copies into and moves
preallocated bounded storage, updates counters, and signals faults without
allocation, blocking, filesystem or network I/O, SQLite, protocol decode, rig
access, logging sinks, or client/GUI locks. The queue implementation review is
[`dependencies/crossbeam-queue-0.3.13.md`](dependencies/crossbeam-queue-0.3.13.md).

Canonical worker-side samples may also feed the bounded spectrum model. Its
private reviewed RustFFT adapter plans one 256–4,096 point power-of-two FFT,
reuses input and scratch buffers, applies a periodic Hann window, and exposes
only owned integer millihertz/mdBFS bins. Fixed chunk, row, bin, allocation,
work, and publication bounds keep visualization independent of callback,
resampling, slot assembly, decode, and client speed. One pending publication
token coalesces when a consumer is slow; oldest rows evict at fixed capacity.
Generation, timeline, or clock invalidation clears overlap/history and marks
the first later row. Detailed behavior is in
[`spectrum-waterfall.md`](spectrum-waterfall.md), and the dependency review is
[`dependencies/rustfft-6.4.1.md`](dependencies/rustfft-6.4.1.md).

## Time boundary

The slot clock samples UTC and monotonic time together, calculates future protocol boundaries, and schedules against monotonic deadlines. It periodically checks the mapping and exposes clock health.

The production receive sampler scopes its monotonic origin to an explicit
non-zero process generation. It brackets each UTC read with monotonic reads
and retains their midpoint in integer milliseconds. The receive monitor
publishes owned snapshots/transitions, a monotonic next-sample due instant, and
latched typed failures for delay, staleness, forward/backward UTC jumps,
timeline regression, suspend/resume-like gaps, and window misalignment.
Unhealthy time converts a complete timeline window into a rejected diagnostic;
it can never return a decoder-ready window. Recovery requires three consecutive
consistent samples and emits an explicit recovered transition. A process
generation change requires a new monitor. Policy details are in
[`receive-clock.md`](receive-clock.md).

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

Schema version 2 normalizes receive windows, one-to-one bounded
audio/timeline/clock diagnostics, and deterministic FT8 decode classifications.
Stable receive identity plus service/process/stream generation, exact device
and configuration, slot, and capture mapping define retry context. Window,
diagnostics, and decodes commit atomically. Queries and retention are sequence
ordered and bounded; raw PCM and bulk waterfall rows are not persisted. See
[`storage.md`](storage.md).

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
