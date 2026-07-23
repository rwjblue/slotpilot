# 0004 — SQLite source of truth and durable outbox

- Status: Accepted
- Date: 2026-07-23

## Context

SlotPilot needs crash-consistent QSO logging, duplicate policy, WSPR spot retention, request idempotency, profile revisions, event history, and retryable external integrations. ADIF cannot serve as a transactional live database.

## Decision

- Use SQLite as the authoritative local operational store.
- Use explicit schema versions and tested forward migrations.
- Commit domain records and pending external effects together through durable outboxes.
- Treat ADIF and WSPRnet as sinks/adapters.
- Never persist active transmit authority.

## Consequences

- Database transaction boundaries become part of QSO completion and queue advancement semantics.
- External delivery can resume safely after restart.
- Import/export and service adapters remain replaceable.

## Revisit when

- measured workload or deployment constraints prove SQLite inadequate;
- a portable bundle format is added and explicitly assigned a different source-of-truth role.
