# 0005 — Attended operation and a single transmit owner

- Status: Accepted
- Date: 2026-07-23

## Context

The desired workflow automates routine FT8 sequencing but the operator remains at the station. Multiple subsystems may want to transmit, and failure paths must not leave PTT asserted or silently resume after restart.

## Decision

- Require explicit, expiring attended-operation authority for transmission.
- Give one `TxSupervisor` exclusive logical ownership of PTT and transmit-plan admission.
- Add an independent maximum-duration watchdog and emergency-unkey path.
- Never restore transmit authority after daemon restart.
- Pause on unexpected operator/rig changes rather than overriding them.

## Consequences

- FT8 and WSPR coordinators submit plans; they do not key rigs.
- Recovery can resume logs and safe outboxes but not transmit operation.
- Safety and fault-injection tests precede automatic QSO sequencing.

## Revisit when

- regulatory or product scope changes; any unattended mode would require a separate explicit design, threat model, and authority mechanism rather than weakening this decision.
