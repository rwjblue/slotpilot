# Architecture decision records

Accepted ADRs define durable project choices. They should explain context, decision, consequences, and conditions that would justify revisiting the choice.

Status values:

- **Proposed**: under discussion; not authoritative;
- **Accepted**: current project decision;
- **Superseded**: replaced by another ADR;
- **Deprecated**: retained for history but no longer recommended.

## Index

- [0001 — GPLv3-or-later and `mfsk-core` boundary](0001-gplv3-and-mfsk-core.md)
- [0002 — One daemon with multiple API clients](0002-daemon-with-multiple-clients.md)
- [0003 — Versioned command/result/event API](0003-versioned-command-event-api.md)
- [0004 — SQLite source of truth and durable outbox](0004-sqlite-and-durable-outbox.md)
- [0005 — Attended operation and a single transmit owner](0005-attended-operation-and-single-tx-owner.md)
- [0006 — Versioned profile composition and session snapshots](0006-versioned-profile-composition.md)
- [0007 — `thiserror` libraries and `anyhow` applications](0007-error-boundaries.md)
- [0008 — Cross-platform design with macOS as primary platform](0008-cross-platform-macos-primary.md)
- [0009 — Initial hardware targets](0009-initial-hardware-targets.md)
- [0010 — GitHub issues and explicit agent handoffs](0010-github-issues-and-agent-handoffs.md)

## Template

```markdown
# NNNN — Title

- Status: Proposed
- Date: YYYY-MM-DD

## Context

## Decision

## Consequences

## Revisit when
```
