# Testing strategy

SlotPilot requires deterministic tests before live-hardware tests because its critical behavior depends on timing, state transitions, and failure handling.

## Test layers

### Domain and policy tests

Pure tests cover:

- callsign normalization without loss of full-call identity;
- bands and frequency arithmetic;
- duplicate-rule matching and explanations;
- caller ordering and score breakdowns;
- profile composition and revision snapshots;
- ADIF field mapping;
- FT8 state-machine transitions;
- schedule collision rules.

### Virtual time

All slot-bound code receives a clock abstraction. Tests can:

- advance to exact FT8 and WSPR boundaries;
- cross UTC midnight;
- inject wall-clock jumps while monotonic time continues;
- simulate late preparation and missed deadlines;
- expire authority without sleeping;
- replay multiple minutes of queue behavior quickly.

`slotpilot-operations` represents UTC and process-local monotonic time as
integer milliseconds sampled together. Future slots are converted to
`MonotonicDeadline` values before scheduling. `ClockMonitor` latches a typed
unhealthy state when UTC and monotonic progress diverge beyond its configured
tolerance. `VirtualClock` advances explicitly and can inject UTC-only jumps;
it never sleeps and does not schedule work or grant authority.

### Protocol fixtures

Maintain reviewed fixtures for:

- ordinary FT8 message classes;
- exchange variants and endings;
- special/compound calls including `W1AW/1`;
- unresolved hashes and unsupported message types;
- WSPR message forms;
- noisy and overlapping recordings;
- encode/decode round trips.

Reference comparisons should record tool/version provenance without importing dependency-specific types into expected public results.

### Fake rig and audio

The test kit provides deterministic implementations that can inject:

- connect/disconnect;
- stale or contradictory readback;
- rejected commands;
- PTT stuck high or delayed;
- unexpected VFO/mode changes;
- audio device disappearance;
- clipping, overrun, underrun, and callback delay;
- sample-clock drift and latency changes.

The `operations` crate owns rig, audio, protocol, and logical
transmit-supervisor traits and values. `slotpilot-testkit` implements them with
in-memory fakes. Faults are queued deterministically; timing-sensitive audio
faults carry a virtual monotonic instant. Protocol samples are placeholders,
not FT8/WSPR algorithms or device output. The emergency-unkey fake records a
logical request and can report stuck PTT, but has no keying mechanism.

### Persistence and crash tests

Test transactional behavior around:

- completed QSO plus duplicate update plus log outbox;
- WSPR spot plus upload outbox;
- request-ID acceptance and conflict;
- restart after each durable step;
- migration from every supported schema version;
- idempotent external receipt handling;
- no persistence of active transmit authority.

### IPC compatibility tests

Keep JSON fixtures for commands, results, errors, events, and snapshots. Test:

- supported version negotiation;
- additive fields;
- unknown event kinds;
- bounded message limits;
- malformed or oversized input;
- reconnect and event cursor behavior;
- local endpoint permissions.

### Fault injection

Every transmit-related subsystem needs focused tests for faults at each boundary before, during, and after PTT. The expected result is explicit inhibition, immediate stop, recoverable state, or retained diagnostics—not an implicit hang.

## Cross-platform matrix

Ordinary CI runs the repository-owned `mise run ci` gate on:

- macOS latest supported release;
- Windows supported release;
- Linux with representative toolchain;
- the repository's exact Rust toolchain pin;
- formatting, lint, unit, integration, fixture, and schema tests.

The initial compile-only workspace has no fixture, schema, audio, or IPC tests;
focused issues add those layers to the same gate as their behavior appears.
Audio and IPC adapters receive platform-specific tests. Hardware tests are
separate and manually authorized.

## Hardware-in-the-loop

Use the ladder in `hardware-support.md`. A hardware test record should include:

- exact radio and firmware;
- Hamlib version/backend;
- interface and audio identifiers;
- dummy load or antenna status;
- power limit;
- expected commands/state;
- actual timing and PTT result;
- operator and date;
- failures and cleanup verification.

## Release safety suite

Before an on-air-capable release:

- emergency stop from every operating state;
- output-device loss during TX;
- rig disconnect during PTT;
- daemon termination and restart;
- wall-clock jump around a slot;
- unexpected mode/frequency movement;
- expired authority between plan and deadline;
- FT8/WSPR schedule collision;
- incomplete database transaction and restart;
- special-call plan validation;
- UTC-midnight logging and duplicate behavior.
