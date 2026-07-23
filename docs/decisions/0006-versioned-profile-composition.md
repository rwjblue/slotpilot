# 0006 — Versioned profile composition and session snapshots

- Status: Accepted
- Date: 2026-07-23

## Context

Operators may use personal, club, special-event, portable, POTA, SOTA, or WWFF contexts. Operator, station, activation, rig, audio, and policy data change at different rates and must not be conflated.

## Decision

- Define separate versioned profiles for operator, station, activation, rig, audio, and operating policy.
- Compose selected revisions into an immutable station-context snapshot at session start.
- Preserve distinct station, operator, and owner callsigns.
- Profile edits create new revisions and do not rewrite active or historical sessions.

## Consequences

- Logs and diagnostics can reconstruct exact operating context.
- Reusable hardware configuration does not need duplication for every activation.
- Profile import never silently grants live authority.

## Revisit when

- experience demonstrates a profile boundary is consistently inseparable from another; migration must preserve historical snapshots.
