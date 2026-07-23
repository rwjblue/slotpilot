# SlotPilot documentation

This directory is the durable design record for the project.

## Product and requirements

- [`vision.md`](vision.md): problem, users, principles, success, and non-goals.
- [`product.md`](product.md): intended operating experience and policy behavior.
- [`requirements.md`](requirements.md): numbered functional, safety, and non-functional requirements.
- [`profiles.md`](profiles.md): operator, station, activation, rig, audio, and operating profiles.
- [`domain.md`](domain.md): stable identifiers, callsigns, radio values, and UTC-slot invariants.
- [`hardware-support.md`](hardware-support.md): initial radio targets and validation expectations.

## Engineering

- [`architecture.md`](architecture.md): process, crate, data, and trust boundaries.
- [`cli-api.md`](cli-api.md): command-line and versioned command/event contract.
- [`safety.md`](safety.md): attended-operation and transmit-safety invariants.
- [`testing.md`](testing.md): deterministic, fault-injection, cross-platform, and hardware test strategy.
- [`storage.md`](storage.md): SQLite schema-v2 migration, bounded receive evidence, pagination, retention, and deferred-field contract.
- [`ipc.md`](ipc.md): user-scoped local transport, framing, permissions, and recovery.
- [`audio-timeline.md`](audio-timeline.md): deterministic receive resampling, slot alignment, tolerances, and fail-closed reset semantics.
- [`receive-clock.md`](receive-clock.md): production UTC/monotonic sampling, receive health latch, alignment gate, and recovery semantics.
- [`spectrum-waterfall.md`](spectrum-waterfall.md): bounded receive FFT, owned bin/row units, reset metadata, and publication coalescing.
- [`development-release.md`](development-release.md): pinned setup, no-op handshake, and unsigned local artifacts.
- [`dependencies/mfsk-core-0.7.4.md`](dependencies/mfsk-core-0.7.4.md): exact offline FT8 dependency review and upgrade procedure.
- [`dependencies/cpal-0.18.1.md`](dependencies/cpal-0.18.1.md): exact receive-audio dependency, stable-identity, platform, and upgrade review.
- [`dependencies/crossbeam-queue-0.3.13.md`](dependencies/crossbeam-queue-0.3.13.md): exact bounded callback-queue dependency and real-time boundary review.
- [`dependencies/rustfft-6.4.1.md`](dependencies/rustfft-6.4.1.md): exact private receive-spectrum FFT dependency and upgrade review.
- [`roadmap.md`](roadmap.md): implementation phases and exit criteria.
- [`work-tracking.md`](work-tracking.md): issue structure, labels, and dependency practices.
- [`backlog/phase-0.md`](backlog/phase-0.md): issue-ready initial work.

## Integrations

- [`integrations/antennabench.md`](integrations/antennabench.md): process-separated AntennaBench contract.
- [`integrations/logging-and-spots.md`](integrations/logging-and-spots.md): ADIF, log sinks, WSPRnet, and durable outboxes.

## Decisions

- [`decisions/README.md`](decisions/README.md): ADR process and index.

Design documents describe the intended system. Accepted ADRs define durable choices. GitHub issues define the exact scope currently authorized for implementation.
