# Contributing to SlotPilot

SlotPilot is currently in its design and bootstrap phase. Contributions should
be tied to a focused GitHub issue. The Phase 0 backlog seeded the initial issue
set but is not a second execution tracker.

## Before starting

Read `AGENTS.md` and the design documents it references. Safety and architecture boundaries are product requirements, not optional implementation guidance.

For durable architectural changes, open or update an architecture decision record under `docs/decisions/` before building multiple layers around the new choice.

The `agent-ready` label records that an issue is bounded and unblocked. It does
not authorize implementation without explicit handoff.

## Change scope

Prefer one coherent change per pull request and one focused issue per pull
request. Avoid creating every planned crate, integration, or user interface in a
single bootstrap change.

Maintained local workflows use Jujutsu (`jj`). Use an issue-scoped bookmark and
do not treat a local-only commit as landed work.

A good initial pull request has:

- one clear goal;
- no physical-radio dependency;
- deterministic tests;
- a documented API or boundary;
- no unrelated formatting or dependency churn.

## Rust conventions

- Rust 2024 edition.
- `thiserror` in libraries; `anyhow` in executable/application entry points.
- Typed domain values in public APIs.
- No blocking or allocation in real-time audio callbacks.
- No unreviewed `unsafe` code. The workspace currently forbids unsafe code.
- No automatic transmission authority after restart or recovery.

## Validation

Use the repository tasks rather than maintaining a separate local command
sequence:

```text
mise run check
mise run ci
```

`mise run check` is the fast loop covering formatting, Clippy with warnings
denied, and workspace tests. `mise run ci` adds the toolchain,
dependency-direction, documentation, and CI-configuration checks and is the
complete landing gate.

Tests that require a physical radio, audio interface, antenna, network service, or operator transmission must not run in ordinary CI.

## Documentation

Update documentation when changing:

- commands, results, events, or exit behavior;
- profile composition or ADIF mapping;
- safety invariants;
- crate responsibilities or dependency direction;
- phase scope or exit criteria;
- external integration contracts.

## License

By contributing, you agree that your contribution is licensed under GPL-3.0-or-later, consistent with the repository license.
