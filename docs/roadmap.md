# Roadmap

The roadmap is ordered by risk reduction. A phase is not complete merely because a happy-path demo exists; its exit criteria must be met.

## Current status

**Phase 0: workspace and contracts — tracked; implementation not started.**

The repository currently contains design documents and project plumbing only.
Implementation is coordinated through
[Phase 0 tracker #1](https://github.com/rwjblue/slotpilot/issues/1).

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

## Phase 1 — offline protocol harness

Goal: validate FT8 and WSPR protocol behavior without live audio or radio control.

Deliverables:

- SlotPilot protocol traits;
- pinned `mfsk-core` adapter;
- FT8 and WSPR WAV/fixture decode;
- waveform generation to files or in-memory buffers;
- golden messages, including special/compound calls such as `W1AW/1`;
- compatibility comparison against trusted reference fixtures.

Exit criteria:

- known recordings decode reproducibly;
- encode/decode round trips cover supported message classes;
- unsupported and unresolved messages are typed and cannot masquerade as automatic transitions;
- dependency types do not escape the protocol crate.

## Phase 2 — receive-only station

Goal: observe live FT8 and WSPR safely.

Deliverables:

- CPAL device discovery and stable identity adapters;
- input stream, bounded ring buffer, resampling, and timestamped windows;
- live FT8 and WSPR decode;
- waterfall/spectrum data model;
- clock-health diagnostics;
- CLI event stream and local persistence of receive data.

Exit criteria:

- configured device loss fails visibly without fallback;
- callbacks meet real-time constraints under stress tests;
- decode windows align under virtual and live clocks;
- no transmit path exists.

## Phase 3 — rig profiles and read-only control

Goal: reliably inspect and verify initial radios without transmitting.

Deliverables:

- persistent rigctld connection and managed/external modes;
- capability probing;
- Hamlib dummy support;
- read-only K4, FT-891/DigiRig, and FTDX10 profiles;
- state-change events and unexpected-change detection.

Exit criteria:

- profile validation clearly reports unsupported operations;
- reconnection and stale-state behavior are tested;
- no PTT or audio-output path exists.

## Phase 4 — manual controlled transmission

Goal: build the safety-critical transmit path before automatic QSO sequencing.

Deliverables:

- output stream and latency calibration;
- immutable transmit plans;
- single-owner transmit supervisor;
- independent PTT watchdog;
- manual FT8 waveform and one-shot WSPR scheduling;
- hard-limited PTT/audio test tools;
- dummy-load and loopback test procedure.

Exit criteria:

- every injected failure dekeys or inhibits safely;
- restart does not restore authority;
- operator emergency stop is independent of ordinary command flow;
- physical testing evidence uses a dummy load or equivalent safe setup.

## Phase 5 — FT8 QSO coordinator and local logging

Goal: conduct one explicitly selected standard QSO safely.

Deliverables:

- typed FT8 message parser and state machine;
- standard exchange variants and retries;
- special-call planning and validation;
- SQLite QSO attempt/event/completion records;
- ADIF export and historical import;
- distinct station/operator/owner fields.

Exit criteria:

- one-and-stop operation is reliable under replay and fault injection;
- UTC-midnight and crash consistency are tested;
- ambiguous or unresolved messages require manual intervention;
- ADIF fixtures preserve required identity and report fields.

## Phase 6 — caller queue and attended automation

Goal: eliminate routine post-QSO clicking while preserving operator control.

Deliverables:

- caller queue and longest-waiting selector;
- duplicate and manual policy engine;
- decision explanations;
- drain-and-stop and drain-then-CQ policies;
- unanswered-CQ and QSO-stage limits;
- pause-after-QSO, pin, skip, select, and resume controls;
- expiring attended-operation authority.

Exit criteria:

- queue ordering and rule results are deterministic and replayable;
- completed QSO and duplicate state commit before queue advancement;
- no rule silently discards a caller without an explanation;
- stop and disarm remain effective at every state.

## Phase 7 — assisted parity and audio lane

Goal: recommend and then optionally select quieter operating space.

Deliverables:

- rolling occupancy model;
- parity and lane scoring with reasons;
- confidence and hysteresis;
- manual pin/override;
- automatic movement only between exchanges;
- collision and hidden-station diagnostics.

Exit criteria:

- recommendations are reproducible from recorded input;
- automatic movement cannot occur mid-QSO;
- operator changes take precedence immediately;
- low-confidence recommendations remain recommendations rather than forced changes.

## Phase 8 — WSPR service and integrations

Goal: complete practical WSPR RX/TX operation and external interoperability.

Deliverables:

- continuous WSPR receive coordinator;
- bounded transmit schedules and percentage policy;
- durable WSPRnet upload;
- WSJT-X-compatible event/output adapters where useful;
- AntennaBench enqueue and transmission-receipt contract;
- explicit future-ready band field in schedules.

Exit criteria:

- upload outage and restart do not lose local spots;
- external request IDs prevent duplicate scheduled transmissions;
- reported actual start/end/PTT times are retained;
- AntennaBench integration remains process-separated.

## Phase 9 — desktop completion and packaging

Goal: ship a coherent cross-platform operator product.

Deliverables:

- desktop workflow for FT8, WSPR, profiles, rules, logs, and diagnostics;
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
