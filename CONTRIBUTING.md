# Contributing to SlotPilot

SlotPilot is currently in its design and bootstrap phase. Contributions should be tied to a scoped issue or a task in `docs/backlog/phase-0.md`.

## Before starting

Read `AGENTS.md` and the design documents it references. Safety and architecture boundaries are product requirements, not optional implementation guidance.

For durable architectural changes, open or update an architecture decision record under `docs/decisions/` before building multiple layers around the new choice.

## Change scope

Prefer one coherent change per pull request. Avoid creating every planned crate, integration, or user interface in a single bootstrap change.

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

The exact commands will be established with the first workspace issue. The intended baseline is:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

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
