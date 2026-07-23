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
- noisy and overlapping recordings;
- encode/decode round trips.

Reference comparisons should record tool/version provenance without importing dependency-specific types into expected public results.
FT8 fixtures are established on the Phase 1 critical path. Equivalent WSPR
fixtures are added in Phase 8 before any WSPR live receive or transmit behavior
is accepted.

The reviewed Phase 1 corpus is `fixtures/ft8/v1/manifest.json`. WSJT-X 3.0.0
is the sole authoritative reference for version 1; supplemental JTDX or
aggressive-decoder observations are explicitly absent. The manifest records the
official artifact checksum, exact `ft8code`, `ft8sim`, and `jt9` settings,
fixture provenance and license, neutral 77-bit protocol facts, file checksums,
decode units, tolerances, recall floors, and permitted extras. Its two compact
recordings are offline-generated data: one clean signal and one noisy,
overlapping window. Passing them makes no broader compatibility or sensitivity
claim.

The message adapter uses the exact reviewed dependency pin recorded in
`dependencies/mfsk-core-0.7.4.md`. Golden tests compare owned message fields and
77 protocol bits, exercise resolved and unresolved compound-call hashes, reject
lossy compound-call encoding, and preserve free text and unsupported structures
as non-resolved outcomes. The `RR73` grid/response collision is classified from
the packed field rather than text, preventing a valid `RR73` locator from
masquerading as an ending.

Offline waveform tests synthesize the independently reviewed ordinary CQ and
Type 4 `CQ W1AW/1` message bits. The canonical output is mono signed 16-bit PCM
at 12,000 Hz: 151,680 samples for the 79-symbol frame or 180,000 samples for an
explicitly placed 15-second slot. Repeated identical requests must be exactly
equal within one platform run; cross-platform CI compares the independent
message bits plus duration, placement, silence, amplitude, clipping, and
RIFF/WAVE byte-order invariants rather than treating platform math-library PCM
bytes as a new golden reference. No test plays or opens the generated audio.

The Phase 1 recording comparison decodes canonical 12,000 Hz mono signed 16-bit
slots over 600–1,800 Hz with a 1.000 sync threshold, normal all-metric search,
no a-priori hints, and a 20-candidate cap. Private dependency tuning environment
variables are rejected so checked-in fixture results cannot change with ambient
process configuration. The clean recording is decoded twice and must produce
the identical normalized sequence. Both recordings must satisfy their manifest
recall floors, permitted-extra lists, and frequency/time/SNR tolerances.

The dependency's raw SNR estimates were 8 dB below the WSJT-X reference for the
clean signal and the first noisy signal, and 6 dB below for the second noisy
signal. The owned adapter therefore applies a documented +8 dB Phase 1
calibration: all three reviewed values then fall within the independently
recorded tolerances. This is bounded conformance evidence, not a claim of
general SNR or sensitivity parity. PCM parsing tests cover truncation, wrong
encoding and bit depth, wrong slot length, lossless in-memory RIFF/WAVE
round-trip, and silence with no decode.

The fixture README defines the intentional refresh process. Ordinary CI parses
the manifest, validates its schema and units, checks every SHA-256 and WAV
header, and rejects missing provenance, duplicate identities, malformed data,
or dependency-specific golden serialization. It does not download fixtures or
execute WSJT-X/JTDX.

`slotpilot-protocol` owns the FT8 fixture-facing vocabulary. A decoded outcome
is exactly one of resolved supported, unresolved hash, unsupported structured,
ambiguous, or free text. Only the resolved variant can pass its checked
conversion. Offline PCM metadata uses integer hertz, channel counts, signed
16-bit sample format, complete frames, and integer duration units. Decode
results sort by owned time offset, audio frequency, canonical text,
classification, and signal report so adapter scheduling cannot define fixture
order.

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

`slotpilot-audio` owns dependency-free receive device identity, configuration,
generation, source-frame position, UTC/monotonic mapping, bounded batch,
canonical FT8 window, health, discontinuity, and fault values.
`slotpilot-operations` owns the consumer port and
`slotpilot-testkit` supplies an in-memory `FakeInputAudio`. Tests enqueue normal
batches and timestamped device loss, overflow, discontinuity, drift, clipping,
callback-delay, and backend faults deterministically without sleeping.
Sequence tests reject generation changes, overlap/regression, unmarked gaps,
and monotonic regression; explicit discontinuities remain visible. The fake
allocates no physical resource and has no device-discovery, playback,
protocol-decode, persistence, rig, PTT, or RF path.

Protocol samples remain deterministic placeholders, not FT8/WSPR algorithms or
device output. The emergency-unkey fake records a logical request and can
report stuck PTT, but has no keying mechanism.

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

## Contact-capable alpha safety suite

Before the Phase 5 contact-capable alpha:

- emergency stop from every operating state;
- output-device loss during TX;
- rig disconnect during PTT;
- daemon termination and restart;
- wall-clock jump around a slot;
- unexpected mode/frequency movement;
- expired authority between plan and deadline;
- incomplete database transaction and restart;
- special-call plan validation;
- UTC-midnight logging and duplicate behavior.

Dummy-load and loopback evidence is required before one bounded,
human-observed FT8 QSO with the designated primary station configuration.
That observation is `human-required` and cannot be replaced by generated or
replay evidence.

## FT8 MVP safety suite

Before the Phase 6 FT8 MVP:

- every contact-capable alpha gate remains satisfied;
- stop, pause, disarm, and emergency stop are exercised from every queue and
  QSO state through the API and CLI;
- the minimum desktop console demonstrates immediate stop and current
  inhibition state;
- authority expiry, retry exhaustion, duplicate policy, durable completion,
  and queue advancement are replayed deterministically;
- one bounded attended CQ run is recorded with the designated primary
  configuration.

## Packaged release safety suite

Before the Phase 9 packaged product release:

- FT8/WSPR schedule collision;
- WSPR storage/upload restart behavior and log separation;
- every claimed initial hardware target completes the hardware test ladder;
- platform permission, device recovery, packaging, migration, backup, and
  rollback smoke tests;
- all contact-capable alpha and FT8 MVP gates remain satisfied.
