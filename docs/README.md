# SlotPilot documentation

This directory is the durable design record for the project.

## Product and requirements

- [`vision.md`](vision.md): problem, users, principles, success, and non-goals.
- [`product.md`](product.md): intended operating experience and policy behavior.
- [`requirements.md`](requirements.md): numbered functional, safety, and non-functional requirements.
- [`profiles.md`](profiles.md): operator, station, activation, rig, audio, and operating profiles.
- [`hardware-support.md`](hardware-support.md): initial radio targets and validation expectations.

## Engineering

- [`architecture.md`](architecture.md): process, crate, data, and trust boundaries.
- [`cli-api.md`](cli-api.md): command-line and versioned command/event contract.
- [`safety.md`](safety.md): attended-operation and transmit-safety invariants.
- [`testing.md`](testing.md): deterministic, fault-injection, cross-platform, and hardware test strategy.
- [`roadmap.md`](roadmap.md): implementation phases and exit criteria.
- [`work-tracking.md`](work-tracking.md): issue structure, labels, and dependency practices.
- [`backlog/phase-0.md`](backlog/phase-0.md): issue-ready initial work.

## Integrations

- [`integrations/antennabench.md`](integrations/antennabench.md): process-separated AntennaBench contract.
- [`integrations/logging-and-spots.md`](integrations/logging-and-spots.md): ADIF, log sinks, WSPRnet, and durable outboxes.

## Decisions

- [`decisions/README.md`](decisions/README.md): ADR process and index.

Design documents describe the intended system. Accepted ADRs define durable choices. GitHub issues define the exact scope currently authorized for implementation.
