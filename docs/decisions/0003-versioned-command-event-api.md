# 0003 — Versioned command/result/event API

- Status: Accepted
- Date: 2026-07-23

## Context

Desktop, CLI, and external clients need stable, machine-readable behavior. Mutating operations may time out at the client while succeeding in the daemon, so retries must not duplicate side effects.

## Decision

- Model service interaction as versioned commands, bounded results, snapshots, and ordered events.
- Use stable request IDs for mutating commands.
- Use JSON for initial wire representations and JSON Lines for event streams.
- Define same-ID/same-command replay and same-ID/different-command conflict behavior.
- Use machine-local IPC by default.

## Consequences

- Wire fixtures and compatibility tests are required before client dependencies grow.
- Prose messages are not stable automation contracts; symbolic codes and typed fields are.
- The CLI can be a thin, complete client rather than a separate code path.

## Revisit when

- performance measurements demonstrate that JSON framing is insufficient;
- multi-user or remote control is explicitly designed with a new security model.
