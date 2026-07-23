# Roadmap

The roadmap is ordered by risk reduction. A phase is not complete merely because a happy-path demo exists; its exit criteria must be met.

## Delivery checkpoints

The roadmap distinguishes engineering capability from a supported product
release:

- **Contact-capable alpha — Phase 5:** a developer/operator can complete and
  log one explicitly selected standard FT8 QSO with one designated, validated
  station configuration. This is a human-observed engineering milestone, not a
  packaged release.
- **FT8 MVP — Phase 6:** an attended operator can run the bounded CQ and caller
  queue workflow through the shared API, CLI, and a minimum desktop operator
  console. This remains a development release until packaging and the complete
  release gates land.
- **Packaged product release — Phase 9:** the coherent cross-platform desktop
  product, complete initial hardware validation, WSPR workflow, migrations,
  backups, documentation, and release safety evidence are ready for
  distribution.

Phases 1 through 6 form the shortest safe FT8 critical path. WSPR
implementation and validation remain product commitments, but they do not gate
the FT8 MVP. Likewise, the FT8 MVP fully validates one designated initial
radio/audio configuration before the remaining initial targets are completed.
These sequencing choices do not weaken the shared architecture, cross-platform
boundaries, single transmit owner, failure behavior, or attended-operation
requirements.

## Current status

**Phase 0: workspace and contracts — complete. Phase 1: offline FT8 protocol
harness — complete. Phase 2 receive-only software work is in progress.**

The repository contains the no-op daemon/CLI handshake, typed contracts,
deterministic test seams, local IPC, initial storage, and complete CI
foundation tracked by
[Phase 0 tracker #1](https://github.com/rwjblue/slotpilot/issues/1). The
[Phase 1 tracker #16](https://github.com/rwjblue/slotpilot/issues/16) records
the owned offline FT8 contract, reviewed dependency/fixtures, message adapter,
PCM synthesis, WAV decode, and bounded conformance evidence. The repository has
owned receive-audio contracts, deterministic fakes, stable input-device
discovery, and an isolated exact-device input adapter with a bounded callback
queue. Pure fixed-point processing aligns/resamples owned batches into
canonical FT8 windows. A production clock sampler and receive-only health gate
reject stale, jumped, delayed, or misaligned mappings. A bounded worker-side
spectrum/waterfall model now has explicit bin/time/magnitude units, reset
metadata, and coalesced publication. SQLite schema version 2 now stores
bounded receive-window context, diagnostics, and exact owned FT8
classifications atomically. The daemon now composes exact receive input,
clock-gated live decode, and atomic ordered decode events behind API version 2;
the CLI has human, JSON, and JSONL receive routes. There is still no audio
output, radio, logging, desktop, WSPR, or transmit implementation.

## Phase 0 — workspace and contracts

Goal: establish compile-time boundaries and deterministic test seams before hardware or DSP work.

Deliverables:

- Rust workspace and minimal boundary crates;
- typed identifiers and domain vocabulary;
- versioned command/result/event envelopes;
- daemon and CLI local-IPC handshake with no station behavior;
- virtual clock;
- fake rig, audio, protocol, and transmit supervisor interfaces;
- initial SQLite migrations and durable command/outbox concepts;
- formatting, linting, test, and documentation CI.

Exit criteria:

- a client can negotiate an API version and request a no-op snapshot;
- mutating request-ID duplicate/conflict behavior is tested;
- time and hardware boundaries are injectable;
- no physical devices or transmissions are possible;
- architecture dependency tests or checks prevent clients from importing hardware internals.

See `backlog/phase-0.md` for issue-ready tasks.

## Phase 1 — offline FT8 protocol harness

Status: complete. Compatibility is limited to the reviewed v1 message and
recording matrix; completion does not claim general WSJT-X/JTDX parity or any
on-air capability.

Goal: validate FT8 protocol behavior without live audio or radio control.

Deliverables:

- the FT8 portion of SlotPilot-owned protocol traits;
- pinned `mfsk-core` FT8 adapter;
- FT8 WAV/fixture decode;
- FT8 waveform generation to files or in-memory buffers;
- golden messages, including special/compound calls such as `W1AW/1`;
- compatibility comparison against trusted reference fixtures.

Exit criteria:

- known FT8 recordings decode reproducibly;
- FT8 encode/decode round trips cover supported message classes;
- unsupported and unresolved messages are typed and cannot masquerade as automatic transitions;
- dependency types do not escape the protocol crate.

WSPR fixtures, encode/decode behavior, and waveform generation are explicitly
deferred to Phase 8. The owned protocol boundary must remain capable of adding
WSPR without exposing dependency types or replacing client contracts.

## Phase 2 — receive-only FT8 station

Status: not started and not authorized by Phase 1 completion.

Goal: observe live FT8 safely through portable audio and time boundaries.

Deliverables:

- CPAL device discovery and stable identity adapters;
- input stream, bounded ring buffer, resampling, and timestamped windows;
- live FT8 decode;
- waterfall/spectrum data model;
- clock-health diagnostics;
- CLI event stream and local persistence of receive data.

Exit criteria:

- configured device loss fails visibly without fallback;
- callbacks meet real-time constraints under stress tests;
- FT8 decode windows align under virtual and live clocks;
- physical audio validation passes on macOS while Windows and Linux preserve
  buildable, tested platform boundaries;
- no transmit path exists.

WSPR live receive is deferred to Phase 8.

Phase 2 software implementation is in progress. The daemon now has an internal,
receive-only composition of exact input ownership, bounded worker processing,
clock-gated live FT8 decode, typed lifecycle and fault handling, and schema-v2
persistence, plus versioned API/CLI observation and control. The
human-required physical input validation remains separate work.

Phase 2 entry was reviewed at the Phase 1 closeout. Its required boundaries are
already explicit: daemon-only hardware ownership, portable stable input-device
identity, allocation-free/nonblocking callbacks, bounded queues, deterministic
resampling and timestamped window alignment, clock-health diagnostics,
receive-only API/CLI events and persistence, device-loss failure without
fallback, and no transmit path. No additional product, safety, licensing,
wire-schema, durable-schema, or architecture decision was discovered during
Phase 1. Implementation still requires an approved tracker and focused issues;
this review creates no authorization or readiness label.

## Phase 3 — primary rig profile and read-only control

Goal: reliably inspect and verify one designated initial station configuration
without transmitting.

Deliverables:

- persistent rigctld connection and managed/external modes;
- capability probing;
- Hamlib dummy support;
- one read-only primary profile selected from the K4, FT-891/DigiRig, and
  FTDX10 initial targets;
- state-change events and unexpected-change detection.

Exit criteria:

- profile validation clearly reports unsupported operations;
- reconnection and stale-state behavior are tested;
- the selected primary configuration completes the read-only physical test
  ladder with recorded human evidence;
- no PTT or audio-output path exists.

The Phase 3 tracker must name the primary radio, control path, audio interface,
and available test equipment. The other initial targets remain required and
move to Phase 7 validation rather than gating the first FT8 contact.

## Phase 4 — manual controlled transmission

Goal: build the safety-critical transmit path before automatic QSO sequencing.

Deliverables:

- output stream and latency calibration;
- immutable transmit plans;
- single-owner transmit supervisor;
- independent PTT watchdog;
- manual FT8 waveform scheduling through the public API and CLI;
- hard-limited PTT/audio test tools;
- dummy-load and loopback test procedure.

Exit criteria:

- every injected failure dekeys or inhibits safely;
- restart does not restore authority;
- operator emergency stop is independent of ordinary command flow;
- physical testing evidence uses a dummy load or equivalent safe setup.

One-shot WSPR scheduling is deferred to Phase 8.

## Phase 5 — contact-capable alpha

Goal: conduct one explicitly selected standard QSO safely.

Deliverables:

- typed FT8 message parser and state machine;
- standard exchange variants and retries;
- special-call planning and validation;
- SQLite QSO attempt/event/completion records;
- ADIF export and historical import;
- distinct station/operator/owner fields;
- complete API and CLI controls for one-and-stop operation, status, pause, and
  emergency stop.

Exit criteria:

- one-and-stop operation is reliable under replay and fault injection;
- UTC-midnight and crash consistency are tested;
- ambiguous or unresolved messages require manual intervention;
- ADIF fixtures preserve required identity and report fields;
- the designated primary configuration completes one bounded, human-observed
  on-air QSO after every dummy-load and failure-path prerequisite passes.

Phase 5 is the first contact-capable engineering milestone. It is not an
end-user release and makes no claim for unattended, multi-radio, WSPR, or
cross-platform operation.

## Phase 6 — FT8 MVP

Goal: eliminate routine post-QSO clicking while preserving operator control.

Deliverables:

- caller queue and longest-waiting selector;
- duplicate and manual policy engine;
- decision explanations;
- drain-and-stop and drain-then-CQ policies;
- unanswered-CQ and QSO-stage limits;
- pause-after-QSO, pin, skip, select, and resume controls;
- expiring attended-operation authority;
- a minimum desktop operator console for station health, decode activity,
  current QSO, caller queue, arm, pause, disarm, emergency stop, and transmit
  inhibition reasons.

Exit criteria:

- queue ordering and rule results are deterministic and replayable;
- completed QSO and duplicate state commit before queue advancement;
- no rule silently discards a caller without an explanation;
- stop and disarm remain effective at every state;
- one-and-stop, drain-and-stop, drain-then-CQ, and continuous-attended policies
  are controllable through the shared API and CLI;
- immediate operator override is available through the API, CLI, and minimum
  desktop console;
- the designated primary station completes the bounded attended CQ workflow
  under recorded human observation.

The desktop framework choice requires its own scoped issue before the minimum
console is implemented. Full profile management, WSPR UI, packaging, backups,
and release polish remain in Phase 9.

## Phase 7 — operating assistance and station expansion

Goal: improve FT8 operating assistance and complete validation of the remaining
initial station targets.

Deliverables:

- rolling occupancy model;
- parity and lane scoring with reasons;
- confidence and hysteresis;
- manual pin/override;
- automatic movement only between exchanges;
- collision and hidden-station diagnostics;
- read-only, state-mutation, dummy-load transmit, and bounded on-air validation
  for the remaining K4, FT-891/DigiRig, or FTDX10 initial targets not selected
  as the primary Phase 3 configuration.

Exit criteria:

- recommendations are reproducible from recorded input;
- automatic movement cannot occur mid-QSO;
- operator changes take precedence immediately;
- low-confidence recommendations remain recommendations rather than forced
  changes;
- each claimed initial hardware configuration has explicit capability,
  failure-path, and human-validation evidence.

## Phase 8 — WSPR service and integrations

Goal: add practical WSPR RX/TX operation and external interoperability on the
proven shared station and transmit boundaries.

Deliverables:

- pinned WSPR protocol adapter, reviewed fixtures, and reference comparisons;
- WSPR waveform generation and live receive decode;
- continuous WSPR receive coordinator;
- one-shot and bounded transmit schedules plus percentage policy;
- local WSPR spot and transmission storage;
- durable WSPRnet upload;
- WSJT-X-compatible event/output adapters where useful;
- AntennaBench enqueue and transmission-receipt contract;
- explicit future-ready band field in schedules.

Exit criteria:

- known WSPR recordings decode reproducibly and supported messages round-trip;
- WSPR and FT8 compete through the same transmit supervisor and cannot overlap;
- upload outage and restart do not lose local spots;
- external request IDs prevent duplicate scheduled transmissions;
- reported actual start/end/PTT times are retained;
- AntennaBench integration remains process-separated.

## Phase 9 — desktop completion and packaging

Goal: ship a coherent cross-platform operator product.

Deliverables:

- desktop workflow for FT8, WSPR, profiles, rules, logs, and diagnostics;
- completed validation evidence for all three initial hardware targets;
- signed/notarized macOS packaging;
- Windows packaging and named-pipe validation;
- Linux packaging and audio-backend validation;
- backups, migrations, release process, and operator documentation.

Exit criteria:

- all desktop mutations use the public API;
- platform permission and device-recovery behavior is tested;
- upgrade and rollback preserve data;
- release artifacts pass safety smoke tests before publication.

## Deferred directions

- WSPR band hopping;
- additional log services;
- PSK Reporter;
- direct Hamlib FFI backend;
- remote operation;
- contest exchanges;
- Fox/Hound;
- additional modes and radios.

Each deferred direction requires a scoped proposal and must preserve the attended and single-owner safety model.
