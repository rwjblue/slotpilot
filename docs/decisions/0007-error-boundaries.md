# 0007 — `thiserror` libraries and `anyhow` applications

- Status: Accepted
- Date: 2026-07-23

## Context

Library callers, state machines, API mapping, and tests need typed errors. Executable startup and top-level orchestration benefit from contextual error reports without forcing every boundary to expose application-specific variants.

## Decision

- Library crates define matchable error enums using `thiserror`.
- Public library APIs do not return `anyhow::Error`.
- Daemon, CLI, desktop composition roots, and one-off operational tooling may use `anyhow` for top-level context and reporting.
- API errors map typed internal errors to stable symbolic codes and structured details.

## Consequences

- Failure behavior can be tested and mapped consistently.
- Application diagnostics can retain context without weakening library contracts.
- Adding a new error variant requires considering API and retry semantics.

## Revisit when

- a crate changes role from reusable library to application-only composition boundary.
